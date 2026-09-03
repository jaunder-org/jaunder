//! The shared machinery behind the ident-keyed XSS gates — [`raw-html-door`] and
//! [`html-sink`].
//!
//! Those gates guard the unescaped DOM boundary from two sides (mint trust and spend
//! trust at the DOM). The traversal lives here once so a fix to the test-code
//! exemption or macro-token walk cannot leave one gate green for the wrong reason.
//! A gate supplies only what is genuinely its own: the roots it scans, the
//! [`population`] it recognises, and the words it fails in.
//!
//! Two layers:
//!
//! - [`traversal::scan`] is the **traversal**: parse, track test-code depth and the enclosing
//!   fn stack, walk macro invocation tokens by hand, and ask whether each occurrence
//!   is in the gate's [`population`].
//! - [`Gate`] is the whole **enumerating gate** on top of it: deny by default,
//!   [`marker_policy::classify`] against the in-source markers, and a [`Report`] supplying the
//!   prose.
//!
//! **A gate reads idents everywhere, by construction.** A population is a set of
//! names, and membership is the same question in ordinary code and inside macro
//! tokens — there is no per-gate hook, so there is no hook a gate can silently fail
//! to implement ("say what you do not look at", #803).
//!
//! **Where the ident is not the whole question, the qualifier decides** (#790). A gate
//! whose population is an associated fn name another type may legitimately share sets
//! [`Gate::owner`]; the walker then resolves each site's qualifier and **suppresses**
//! the ones that belong to some other type. Two properties make that safe rather than
//! a hole:
//!
//! - **[`visit_ident`] stays the sole recorder.** Resolution only ever suppresses. A
//!   `fn` ident is not a [`syn::Path`], nor is a method-call ident or a macro token, so
//!   recording from a path hook would silently drop every definition site — including
//!   the guarded door's own. It also means a site cannot be counted twice, and that
//!   `owner: None` scans with no suppression at all: the suppression set is simply
//!   empty.
//! - **Unresolvable means in-population.** A qualifier the gate cannot pin — glob
//!   import, generic parameter, unqualified call, macro body — stays policed. Obscuring
//!   a qualifier buys a gate failure, not an exemption.
//!
//! Deciding membership this way is **structural**: it identifies the door rather than
//! exempting a site from it, so ADR-0085 principle 3 is not in play. #778 conflated the
//! two and deleted a qualifier check as a pattern exemption, which left the codebase
//! carrying markers on a provably harmless population. See
//! `docs/adr/0110-gate-population-membership-is-structural.md`.
//!
//! Macro bodies are deliberately **not** resolved — `walk_macro_tokens` sees a flat
//! token stream, and under the rule above not resolving is fail-closed. A
//! path-qualifier read three tokens to the left remains an available seam, since
//! `walk_macro_tokens` already materialises the flat sibling stream (the index is
//! one `.enumerate()` away).
//!
//! [`visit_ident`]: syn::visit::Visit::visit_ident
//!
//! **Exemptions are markers, not a list** (#778). A site is exempt when the line
//! *immediately above* it carries `// <gate-step>:allow <reason>`. The key is one
//! line, so it cannot absorb a second site the way a fn-keyed entry did; it moves
//! with the code under rename and refactor; and the exempt set is **derived** from
//! the tree rather than declared beside the rule, which removes the whole class of
//! staleness a declared list creates.
//!
//! The position is not a matter of taste: a *trailing* marker is relocated by
//! `rustfmt` (below an opening brace) and by `leptosfmt` (in a `view!` body), so
//! only the line above is stable, and only the line above is honored.
//!
//! **Unreadable classes inherent to this scan** (ADR-0085's honesty obligation;
//! each gate states the ones specific to *its* idents on top of these):
//!
//! 1. **Only for an owner-configured gate** (see [`Gate::owner`]), three ways a
//!    qualifier can mislead resolution, all fail-**open** (#790):
//!    a rename of a rename — [`resolution::owner_aliases`] harvests a single
//!    `use …Owner as X`, so a rename *of that rename* in a third module evades;
//!    a renaming re-export living **outside** the gate's roots, which is never
//!    harvested at all, so a use site inside them resolves to the alias's own name
//!    and is suppressed; and a free `fn` nested inside another type's `impl` method
//!    body, which the enclosing-`impl` lookup attributes to that type. None has a
//!    live instance; the first two are why a gate's roots must cover every tree it
//!    claims to police. For a gate with no owner there is nothing to resolve, and a
//!    `use … as` rename simply evades ident matching outright — `syn` has no name
//!    resolution, and before #790 that was this class's whole content.
//! 2. Tokens inside an *attribute* macro's argument list are not walked; only
//!    [`syn::Macro`] invocations are. Macro **expansions** are never seen either,
//!    which is deliberate: only author-written tokens are in the population.
//! 3. There is no call graph, so a member reached through a helper is attributed to
//!    the helper, not to the caller that supplied the untrusted value. The scan can
//!    detect; it cannot attribute.
//! 4. A marker is **trusted, not verified**. The gate checks that a reason exists
//!    and that the marker still points at a site; it can never check that the
//!    reason is true. That is inherent to any written exemption — `cov:ignore` has
//!    it too (ADR-0050 records those as permanent blind spots) — and it is why the
//!    set must stay small enough to re-read.
//! 5. A marked site is exempt regardless of what value flows *into* it. Narrowing
//!    the exemption from a function to a line shrinks that window (class 3 above
//!    is why it cannot be closed) but does not shut it.
//! 6. The test-code exemption is decided by a **substring**, not by parsing the
//!    `cfg` predicate: [`traversal::is_test_cfg`] asks whether the attribute's tokens mention
//!    `test` and not `not`. So `#[cfg(feature = "test-utils")]` reads as test code
//!    and its members are dropped from the population entirely — no marker owed,
//!    no census row. Nothing under the policed roots currently matches *and*
//!    encloses a gate ident, so there is no live hole; it is recorded because a
//!    **pattern** on an attribute's text is deciding an exemption, which is what
//!    ADR-0085 principle 3 forbids — and unlike deciding *membership* from a
//!    resolved qualifier (#790, which is structural), this really is an exemption
//!    granted by pattern. The marker work also made it load-bearing in a second
//!    place (`test_ranges`, which suppresses orphan reports). Parsing the predicate
//!    would close it.
//!
//! A `syn` parse failure is a **hard error** everywhere (ADR-0085 principle 6): a
//! file we cannot walk could hide a member, and a gate that quietly shrinks its own
//! population reports green for the one reason it must never report green.
//!
//! [`population`]: Gate::population
//! [`raw-html-door`]: crate::steps::raw_html_door_check
//! [`html-sink`]: crate::steps::html_sink_check

mod marker_policy;
mod orchestration;
mod resolution;
mod traversal;

pub use orchestration::{Gate, Report, run_scan};
