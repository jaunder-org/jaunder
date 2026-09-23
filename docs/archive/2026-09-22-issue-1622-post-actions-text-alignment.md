# Issue #1622 — Post Actions text alignment

## Outcome

The trusted Post Actions control reads as part of the Post header because its
visible “Actions” label shares the published-time text baseline. The protected
button footprint, author/date typography, and disclosure behavior remain
unchanged.

## Load-bearing decisions

- The alignment target is the **text baseline** of “Actions” and the published
  time, not the geometric center or outer edge of the button box.
- The author name, handle, source, and published time retain their existing
  baseline relationship. The whole Post header does not switch to center
  alignment.
- The viewer-independent Actions slot supplies an invisible typographic baseline
  that matches the trusted trigger's label metrics. Normal flex baseline layout
  then aligns the published time with that probe.
- The probe uses the same shared label text and Jaunder-owned literal font,
  line-height, padding, and border metrics as the trusted trigger. Those metrics
  do not resolve through Theme Package-overridable custom properties. It is not
  implemented as an empirical translate, transform, margin, or per-theme offset.
- The probe remains invisible, non-interactive, and excluded from the
  accessibility tree. Protected overflow clipping and off-canvas indentation
  prevent any text paint, including theme-supplied stroke or shadow effects. It
  must not expose a duplicate “Actions” control or name.
- ADR-0188's protected slot remains a fixed 72×32 in-flow box with its unique
  `anchor-name`; owner enhancement adds no geometry and anonymous rendering
  reserves the same footprint.
- The alignment applies to every owned-Post surface that uses the shared Post
  header, including Local, author, tag, and permalink routes under built-in and
  custom Theme Package presentation.
- Theme Package CSS cannot directly restyle or reveal the protected probe. The
  trusted trigger and popover remain outside the Theme Package surface.

## Acceptance

- In an owned Post header, the rendered “Actions” label and published time have
  matching visual baselines at desktop and narrow viewports.
- Author name, handle, source, and time retain their existing mutual baseline
  alignment; no whole-row center-alignment regression is introduced.
- The Actions button remains 72×32, and its border box stays anchored to the
  same reserved slot without overlap, clipping, or horizontal overflow.
- Local, author, tag, and permalink routes produce the same baseline relation.
- Built-in Studio and an adversarial custom Theme Package that changes public
  typography variables and text paint both retain the alignment and invisible
  probe required by ADR-0188.
- Anonymous and owner rendering reserve identical header geometry, and the
  invisible probe contributes no accessibility-tree content or focus target.
- Existing native popover placement, keyboard focus, dismissal, and Post/action
  association tests continue to pass.
- A focused browser regression inserts test-only, zero-size inline-block probes
  with `vertical-align: baseline` immediately after the visible label and time
  text, then compares the probes' bottom Y coordinates with coordinate-rounding
  tolerance only. Inline-important test styles protect the probes from a custom
  theme; no production probe element is added.
- That regression separately proves the protected slot and trusted trigger have
  coincident 72×32 border boxes. Screenshots, calibrated gaps, and font-specific
  empirical offsets are not baseline or geometry oracles.
- Transient Before/After screenshots show the same deterministic owned Post on a
  permalink at 1440×600, with no state or presentation difference beyond the
  corrected text baseline.

## Boundaries

- This issue does not resize, relabel, or redesign the Actions button or menu.
- It does not change action authorization, available actions, popover behavior,
  CSS Anchor Positioning, browser support, or Theme Package contracts.
- It does not otherwise restyle the Post header, author metadata, published
  time, title, or body.
