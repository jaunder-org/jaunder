import { randomBytes } from "crypto";

type OtlpValue =
  | { stringValue: string }
  | { intValue: string }
  | { doubleValue: number }
  | { boolValue: boolean };

type OtlpAttribute = {
  key: string;
  value: OtlpValue;
};

type OtlpEvent = {
  name: string;
  timeUnixNano: string;
  attributes?: OtlpAttribute[];
};

type OtlpSpan = {
  traceId: string;
  spanId: string;
  parentSpanId?: string;
  name: string;
  kind: number;
  startTimeUnixNano: string;
  endTimeUnixNano: string;
  attributes?: OtlpAttribute[];
  events?: OtlpEvent[];
};

export type TraceContext = {
  traceId: string;
  parentSpanId?: string;
};

const DEFAULT_OTLP_HTTP_ENDPOINT = "http://127.0.0.1:4318/v1/traces";
const SPAN_KIND_INTERNAL = 1;
const SPAN_KIND_CLIENT = 3;

function toNanoString(msSinceEpoch: number): string {
  return String(BigInt(Math.round(msSinceEpoch * 1_000_000)));
}

function bytesToHex(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("hex");
}

function randomHex(byteLength: number): string {
  return bytesToHex(randomBytes(byteLength));
}

function parseTraceParent(traceParent: string): TraceContext | null {
  const parts = traceParent.trim().split("-");
  if (parts.length !== 4) return null;
  const [version, traceId, parentSpanId] = parts;
  if (
    version.length !== 2 ||
    traceId.length !== 32 ||
    parentSpanId.length !== 16
  ) {
    return null;
  }
  if (!/^[0-9a-f]{2}$/i.test(version)) return null;
  if (!/^[0-9a-f]{32}$/i.test(traceId)) return null;
  if (!/^[0-9a-f]{16}$/i.test(parentSpanId)) return null;
  return {
    traceId: traceId.toLowerCase(),
    parentSpanId: parentSpanId.toLowerCase(),
  };
}

export function traceContextFromEnvironment(): TraceContext {
  const traceParent = process.env.JAUNDER_E2E_TRACEPARENT;
  if (traceParent) {
    const parsed = parseTraceParent(traceParent);
    if (parsed) return parsed;
  }

  const explicitTraceId = process.env.JAUNDER_E2E_TRACE_ID;
  if (explicitTraceId && /^[0-9a-f]{32}$/i.test(explicitTraceId)) {
    return { traceId: explicitTraceId.toLowerCase() };
  }

  return { traceId: randomHex(16) };
}

export function otlpAttribute(
  key: string,
  value: string | number | boolean | bigint | null | undefined,
): OtlpAttribute | null {
  if (value === null || value === undefined) return null;
  if (typeof value === "string") return { key, value: { stringValue: value } };
  if (typeof value === "boolean") return { key, value: { boolValue: value } };
  if (typeof value === "bigint")
    return { key, value: { intValue: value.toString() } };
  if (Number.isInteger(value))
    return { key, value: { intValue: String(value) } };
  return { key, value: { doubleValue: value } };
}

export type SpanInput = {
  traceContext: TraceContext;
  name: string;
  parentSpanId?: string;
  kind?: "internal" | "client";
  startMs: number;
  endMs: number;
  attributes?: OtlpAttribute[];
  events?: OtlpEvent[];
};

export function makeEvent(
  name: string,
  whenMs: number,
  attributes?: OtlpAttribute[],
): OtlpEvent {
  return {
    name,
    timeUnixNano: toNanoString(whenMs),
    ...(attributes && attributes.length > 0 ? { attributes } : {}),
  };
}

export function buildSpan(input: SpanInput): OtlpSpan {
  const span: OtlpSpan = {
    traceId: input.traceContext.traceId,
    spanId: randomHex(8),
    ...(input.parentSpanId || input.traceContext.parentSpanId
      ? { parentSpanId: input.parentSpanId ?? input.traceContext.parentSpanId }
      : {}),
    name: input.name,
    kind: input.kind === "client" ? SPAN_KIND_CLIENT : SPAN_KIND_INTERNAL,
    startTimeUnixNano: toNanoString(input.startMs),
    endTimeUnixNano: toNanoString(input.endMs),
  };

  if (input.attributes && input.attributes.length > 0) {
    span.attributes = input.attributes;
  }
  if (input.events && input.events.length > 0) {
    span.events = input.events;
  }
  return span;
}

export async function exportSpans(spans: OtlpSpan[]): Promise<void> {
  if (spans.length === 0) return;

  const endpoint =
    process.env.JAUNDER_E2E_OTLP_HTTP_ENDPOINT ?? DEFAULT_OTLP_HTTP_ENDPOINT;
  const shouldExport =
    process.env.JAUNDER_E2E_OTLP_HTTP_ENDPOINT !== undefined ||
    process.env.JAUNDER_E2E_TRACEPARENT !== undefined;
  if (!shouldExport) return;

  const body = {
    resourceSpans: [
      {
        resource: {
          attributes: [
            {
              key: "service.name",
              value: { stringValue: "jaunder-e2e" },
            },
            {
              key: "service.namespace",
              value: { stringValue: "end2end" },
            },
          ],
        },
        scopeSpans: [
          {
            scope: {
              name: "jaunder.end2end",
            },
            spans,
          },
        ],
      },
    ],
  };

  const response = await fetch(endpoint, {
    method: "POST",
    headers: {
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });

  if (!response.ok) {
    const payload = await response.text();
    throw new Error(`OTLP export failed (${response.status}): ${payload}`);
  }
}
