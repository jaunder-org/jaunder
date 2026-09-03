//! Host-testable one-flight state machine for client diagnostics.
//!
//! [`Reporter`] logs every event locally before it considers transport, drops
//! concurrent delivery without a queue, and lets only the active transport's
//! completion callback release its flight slot.

use std::cell::Cell;
use std::rc::Rc;

use common::client_telemetry::{
    self, ClientErrorContext, ClientErrorKind, ClientSourceKind, ClientTelemetryEvent,
};

/// Completion callback that releases a reporter's single flight slot.
pub type Completion = Box<dyn FnOnce() + 'static>;

/// Starts best-effort delivery and invokes the callback exactly once for every
/// terminal outcome.
pub trait Transport {
    fn send(&self, event: ClientTelemetryEvent, on_complete: Completion);
}

/// Local warning sink invoked before central delivery is considered.
pub trait ConsoleSink {
    fn warn(&self, event: &ClientTelemetryEvent);
}

/// Synchronous one-flight reporter over an injected transport.
pub struct Reporter<T> {
    transport: T,
    console: Box<dyn ConsoleSink>,
    in_flight: Rc<Cell<bool>>,
}

impl<T> Reporter<T>
where
    T: Transport,
{
    /// Builds an isolated reporter with an empty flight slot.
    #[must_use]
    pub fn new<C>(transport: T, console: C) -> Self
    where
        C: ConsoleSink + 'static,
    {
        Self {
            transport,
            console: Box::new(console),
            in_flight: Rc::new(Cell::new(false)),
        }
    }

    /// Warns locally, then starts at most one best-effort delivery.
    pub fn report_swallowed(
        &self,
        kind: ClientErrorKind,
        context: ClientErrorContext,
        source_kind: ClientSourceKind,
    ) {
        let event = ClientTelemetryEvent {
            version: client_telemetry::WIRE_VERSION,
            kind,
            context,
            source_kind,
        };
        self.console.warn(&event);

        if self.in_flight.replace(true) {
            return;
        }

        let in_flight = Rc::clone(&self.in_flight);
        self.transport.send(
            event,
            Box::new(move || {
                in_flight.set(false);
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;

    #[derive(Default)]
    struct ManualState {
        trace: RefCell<Vec<&'static str>>,
        console_events: RefCell<Vec<ClientTelemetryEvent>>,
        sent_events: RefCell<Vec<ClientTelemetryEvent>>,
        completions: RefCell<VecDeque<Completion>>,
    }

    #[derive(Clone, Default)]
    struct ManualTransport(Rc<ManualState>);

    impl ManualTransport {
        fn complete(&self) {
            self.0
                .completions
                .borrow_mut()
                .pop_front()
                .expect("one transport completion")();
        }

        fn sent_events(&self) -> Vec<ClientTelemetryEvent> {
            self.0.sent_events.borrow().clone()
        }

        fn console_events(&self) -> Vec<ClientTelemetryEvent> {
            self.0.console_events.borrow().clone()
        }

        fn trace(&self) -> Vec<&'static str> {
            self.0.trace.borrow().clone()
        }
    }

    impl Transport for ManualTransport {
        fn send(&self, event: ClientTelemetryEvent, on_complete: Completion) {
            self.0.trace.borrow_mut().push("transport");
            self.0.sent_events.borrow_mut().push(event);
            self.0.completions.borrow_mut().push_back(on_complete);
        }
    }

    #[derive(Clone, Default)]
    struct ImmediateTransport(Rc<ManualState>);

    impl Transport for ImmediateTransport {
        fn send(&self, event: ClientTelemetryEvent, on_complete: Completion) {
            self.0.trace.borrow_mut().push("transport");
            self.0.sent_events.borrow_mut().push(event);
            on_complete();
        }
    }

    struct CapturingConsole(Rc<ManualState>);

    impl ConsoleSink for CapturingConsole {
        fn warn(&self, event: &ClientTelemetryEvent) {
            self.0.trace.borrow_mut().push("console");
            self.0.console_events.borrow_mut().push(*event);
        }
    }

    fn reporter() -> (Reporter<ManualTransport>, ManualTransport) {
        let transport = ManualTransport::default();
        let console = CapturingConsole(Rc::clone(&transport.0));
        (Reporter::new(transport.clone(), console), transport)
    }

    fn report_one(reporter: &Reporter<ManualTransport>) {
        reporter.report_swallowed(
            ClientErrorKind::Storage,
            ClientErrorContext::ThemeStorageRead,
            ClientSourceKind::StorageUnavailable,
        );
    }

    fn report_two(reporter: &Reporter<ManualTransport>) {
        reporter.report_swallowed(
            ClientErrorKind::Dialog,
            ClientErrorContext::PublishConfirm,
            ClientSourceKind::DialogUnavailable,
        );
    }

    #[test]
    fn report_logs_before_send_and_returns_synchronous_unit() {
        let (reporter, transport) = reporter();

        reporter.report_swallowed(
            ClientErrorKind::Storage,
            ClientErrorContext::ThemeStorageRead,
            ClientSourceKind::StorageUnavailable,
        );

        assert_eq!(transport.trace(), vec!["console", "transport"]);
        assert_eq!(
            transport.sent_events(),
            vec![ClientTelemetryEvent {
                version: client_telemetry::WIRE_VERSION,
                kind: ClientErrorKind::Storage,
                context: ClientErrorContext::ThemeStorageRead,
                source_kind: ClientSourceKind::StorageUnavailable,
            }]
        );
        assert_eq!(transport.console_events(), transport.sent_events());
    }

    #[test]
    fn delayed_completion_logs_and_drops_concurrent_then_reopens_without_queueing() {
        let (reporter, transport) = reporter();

        report_one(&reporter);
        report_two(&reporter);
        assert_eq!(transport.trace(), vec!["console", "transport", "console"]);
        assert_eq!(transport.console_events().len(), 2);
        assert_eq!(transport.sent_events().len(), 1);

        transport.complete();
        assert_eq!(transport.sent_events().len(), 1);
        assert_eq!(transport.console_events().len(), 2);
        report_two(&reporter);

        assert_eq!(transport.sent_events().len(), 2);
        assert_eq!(
            transport.sent_events()[1].context,
            ClientErrorContext::PublishConfirm
        );
    }

    #[test]
    fn inline_completion_releases_before_return_without_recursion_or_duplicate_sends() {
        let transport = ImmediateTransport::default();
        let console = CapturingConsole(Rc::clone(&transport.0));
        let reporter = Reporter::new(transport.clone(), console);

        reporter.report_swallowed(
            ClientErrorKind::Storage,
            ClientErrorContext::ThemeStorageRead,
            ClientSourceKind::StorageUnavailable,
        );

        assert_eq!(transport.0.console_events.borrow().len(), 1);
        assert_eq!(transport.0.sent_events.borrow().len(), 1);

        reporter.report_swallowed(
            ClientErrorKind::Dialog,
            ClientErrorContext::PublishConfirm,
            ClientSourceKind::DialogUnavailable,
        );

        assert_eq!(
            transport.0.trace.borrow().as_slice(),
            ["console", "transport", "console", "transport"]
        );
        assert_eq!(transport.0.console_events.borrow().len(), 2);
        assert_eq!(transport.0.sent_events.borrow().len(), 2);
        assert_eq!(
            transport.0.sent_events.borrow()[1].context,
            ClientErrorContext::PublishConfirm
        );
    }
}
