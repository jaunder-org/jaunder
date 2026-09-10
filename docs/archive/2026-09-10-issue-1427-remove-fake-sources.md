# Remove fabricated sidebar Sources

Issue: #1427

## Outcome

Jaunder's production sidebar no longer presents made-up Sources as if they were
real followed content. Anonymous and authenticated viewers see the remaining
navigation without a Sources section.

## Load-bearing decisions

- Remove the entire static Sources section, including its heading, add control,
  and fabricated rows.
- Apply the removal to both the server-projected anonymous sidebar and the
  reactive authenticated sidebar so the two surfaces do not diverge.
- Do not replace the section with an empty state. Jaunder must not imply that
  source-following is currently available.
- Real followed-source navigation may return only with the corresponding product
  capability and real data; this change creates no placeholder contract.

## Acceptance

- An anonymous viewer's sidebar contains no Sources heading, add control, or
  fabricated source rows.
- An authenticated viewer's sidebar contains no Sources heading, add control, or
  fabricated source rows.
- Existing brand, search, primary navigation, active-route indication, and
  authenticated footer behavior remain unchanged.
- The anonymous server projection and client-rendered anonymous sidebar remain
  identical across the change.

## Boundaries

- No source-following, source discovery, ingestion, or persistence is added.
- No replacement sidebar content or general sidebar redesign is included.
- No navigation, authentication, registration-policy, or footer behavior
  changes.
