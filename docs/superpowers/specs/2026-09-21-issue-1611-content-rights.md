# Issue #1611: Copyright Declarations and Content Licenses

## Outcome

Every publicly presented Post identifies its rights holder, creation year, and
Content License. Each User can choose a publication-wide Content License; the
choice is visible on the web and represented in every public Syndication Feed
format without altering authored Post content.

## Load-bearing decisions

- A User has one current Content License for all their Posts. It defaults to All
  Rights Reserved for existing and new Users.
- Changing the Content License intentionally applies retroactively to every Post
  by that User. There is no per-Post override or historical license snapshot.
- The closed choices come from the official Creative Commons catalog. Their
  exact public label, SPDX identity, and canonical URL are:

  | Label               | SPDX identifier   | Canonical URL                                        |
  | ------------------- | ----------------- | ---------------------------------------------------- |
  | All Rights Reserved | —                 | —                                                    |
  | CC0 1.0             | `CC0-1.0`         | `https://creativecommons.org/publicdomain/zero/1.0/` |
  | CC BY 4.0           | `CC-BY-4.0`       | `https://creativecommons.org/licenses/by/4.0/`       |
  | CC BY-SA 4.0        | `CC-BY-SA-4.0`    | `https://creativecommons.org/licenses/by-sa/4.0/`    |
  | CC BY-ND 4.0        | `CC-BY-ND-4.0`    | `https://creativecommons.org/licenses/by-nd/4.0/`    |
  | CC BY-NC 4.0        | `CC-BY-NC-4.0`    | `https://creativecommons.org/licenses/by-nc/4.0/`    |
  | CC BY-NC-SA 4.0     | `CC-BY-NC-SA-4.0` | `https://creativecommons.org/licenses/by-nc-sa/4.0/` |
  | CC BY-NC-ND 4.0     | `CC-BY-NC-ND-4.0` | `https://creativecommons.org/licenses/by-nc-nd/4.0/` |

  All Rights Reserved is a rights statement rather than a license and therefore
  has neither an SPDX identifier nor a license URL.

- A Copyright Declaration is Post metadata composed from the Post's immutable
  creation year, the author's current Display Name when present (otherwise
  canonical Username), and the author's current Content License.
- Every public Post occurrence carries its own declaration. Multi-Post pages do
  not aggregate years or rights holders into a page-level declaration.
- Public Syndication Feed items carry equivalent rights metadata separately from
  their authored and rendered content. The exact wire contract is:
  - Atom entries contain the native text construct
    `<rights>DECLARATION</rights>`; RFC 4287 defines an omitted `type` as text.
    A Creative Commons choice also adds
    `<link rel="license" type="text/html" href="CANONICAL_URL"/>`; All Rights
    Reserved adds no license link.
  - RSS items contain `<dc:rights>DECLARATION</dc:rights>` under
    `xmlns:dc="http://purl.org/dc/elements/1.1/"`. A Creative Commons choice
    also adds `<creativeCommons:license>CANONICAL_URL</creativeCommons:license>`
    under
    `xmlns:creativeCommons="http://backend.userland.com/creativeCommonsRssModule"`;
    All Rights Reserved adds no license element.
  - JSON Feed items contain
    `_jaunder: {"copyright":"© YEAR NAME", "rights":"LABEL","license":LICENSE}`.
    `LICENSE` is `null` for All Rights Reserved; otherwise it is
    `{"spdx_id":"SPDX_ID","url":"CANONICAL_URL"}`. JSON Feed 1.1 permits
    underscore-prefixed extension objects and requires unaware readers to ignore
    them.
- `DECLARATION` is exactly `© YEAR NAME · LABEL`, using the labels in the table
  above. HTML displays the same text and links only `LABEL` for Creative Commons
  choices.
- AtomPub Collections remain source-oriented editing representations and do not
  expose Copyright Declarations or Content Licenses.
- The durable architecture decision is recorded in
  `docs/adr/drafts/user-wide-current-content-rights.md`.

## Acceptance

- Account settings present a Content License choice with All Rights Reserved as
  the default, the seven Creative Commons choices above, canonical license
  links, and an explicit explanation that changes affect all existing Posts.
- A User's selection persists on SQLite and PostgreSQL and survives restart,
  backup, and restore through the repository's ordinary storage contracts.
- Local, User, Site Tag, User Tag, and permalink presentations show each public
  Post's declaration as `© <creation year> <current author name> · <rights>`;
  Creative Commons rights labels link to their canonical license text.
- A Display Name or Content License change is reflected on every affected public
  Post presentation. In the same durable mutation, it enqueues the affected
  Site, Site Tag, User, and User Tag feed events; the existing publisher
  generation gate regenerates those representations before issuing
  duplicate-safe, at-least-once WebSub Publish Pings. A failed enqueue prevents
  the profile/configuration mutation from committing.
- Site, Site Tag, User, and User Tag feeds in Atom, RSS, and JSON Feed carry the
  equivalent declaration and license identity for every item. Feed Post bodies,
  summaries, and titles remain byte-for-byte independent of that metadata.
- AtomPub Service Documents, Collections, and Member Entries remain unchanged.
- Both storage backends have parity tests for defaults, accepted values,
  persistence, updates, and transactional feed-event creation. Serializer tests
  assert the exact XML/JSON contract above for All Rights Reserved and every
  Creative Commons mapping.
- The account-settings read/update endpoint has dual-backend HTTP integration
  coverage for authentication, validation, persistence, and mutation outcomes,
  plus end-to-end coverage for choosing a license and observing its retroactive
  public effect.
- The existing Local visual snapshot is intentionally updated for the new Post
  metadata. Review-only visual proof—not additional committed snapshot states—
  compares Local and permalink before/after presentation at desktop and compact
  viewports, including All Rights Reserved and a linked Creative Commons choice
  under the built-in theme.

## Boundaries

- No operator- or site-owned copyright notice is introduced.
- No arbitrary license text, URL, SPDX expression, or non-CC license is
  accepted.
- No per-Post license control, license history, or automatic legal advice is
  added.
- Authenticated Home, settings, and administration screens do not gain a global
  legal footer; settings change only where the Content License is managed.
- This work does not redefine Post creation, publication, scheduling, audience,
  deletion, or revision semantics.
