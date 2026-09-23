# Fit Post images to available width (#1630)

## Outcome

Oversized images in a Post body fit within the available Post width on narrow
and wide screens, without horizontal overflow or distorted proportions. Smaller
images retain their natural size. The original Media bytes and URLs remain
unchanged.

## Load-bearing decisions

- Apply a shared presentation default to Post-body images, independent of how
  the image entered the rendered Post (Markdown, Org, or HTML). It applies on
  Home under the shared built-in presentation, and on Local and public Post
  permalinks under built-in and custom public themes. Home never loads custom
  public theme CSS (ADR-0184).
- Constrain display size to the available Post-body width and preserve the
  intrinsic aspect ratio, including for authored images carrying width/height
  attributes. Do not enlarge an image that already fits.
- Keep the rule overridable by an intentional custom Theme Package stylesheet;
  do not force it with `!important`. The default behavior must not depend on
  image source or theme selection.
- This is display sizing only: no generated image variants, upload processing,
  storage changes, request-dependent markup, new Media routes, or `srcset`.

## Acceptance

- Deterministic loaded-image fixtures and browser geometry assertions at mobile
  and desktop widths prove that an oversized image fits within the Post body's
  content width, a smaller image keeps its intrinsic width, authored
  width/height attributes do not distort its intrinsic aspect ratio, and neither
  the Post nor the document gains horizontal overflow.
- The default is verified on Home with shared built-in styling and on Local and
  a permalink with a built-in and a custom public theme. A custom theme with no
  image rule inherits the default, while an intentional image rule overrides it.
- Rendering evidence shows Markdown, Org, and HTML images end up inside the same
  Post-body wrapper; one representative format receives focused browser geometry
  coverage rather than a full format × route × theme matrix.
- Non-baseline manual review artifacts: comparable **Before** and **After**
  permalink screenshots at mobile and desktop widths with the same content,
  built-in theme, and account state. Keep paths and conditions in the review/PR
  handoff, outside the repository and outside Playwright's visual snapshot
  population; automated regression evidence is geometry assertions, not new
  screenshot baselines.
- No change to serialized Post HTML, Syndication Feeds, AtomPub, Media records
  or stored image bytes.

## Boundaries

- No image optimization, lazy loading, cropping, lightbox, content format
  rewrite, or sizing rule for non-Post-body images (logo, header, avatars, Theme
  Package assets).
- No new architectural decision: the existing shared Style Contract and theme
  stylesheet layering remain authoritative.
