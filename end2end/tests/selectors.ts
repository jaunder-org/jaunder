/**
 * Named selector constants for the e2e suite (#263).
 *
 * High-frequency CSS selector strings route through `SEL` — a single source of
 * truth and a uniform quote style, so a markup rename or a typo touches one
 * file with compiler help. One-off / rarely-repeated selectors stay inline at
 * their call sites.
 */

export const SEL = {
  /** Save-summary panel shown after a successful compose/publish. */
  saveSummary: ".j-save-summary",
  /** Post-composer body textarea. */
  postBody: 'textarea[name="body"]',
  /** Post summary textarea (compose + edit). Keyed on `name` since #568 — the
   * shared `<ValidatedTextarea>` emits no id. */
  postSummary: 'textarea[name="summary"]',
  /** Publish/unpublish submit button; `value` is the boolean string. */
  publishButton: (value: string) => `button[name="publish"][value="${value}"]`,
  /** A `.j-seg` format-toggle button, by its visible label. The label is a
   * literal union, not `string`: a casing typo (`"org"`) would otherwise
   * compile and fail as a locator timeout. */
  formatButton: (label: "Markdown" | "Org") =>
    `.j-seg button:has-text("${label}")`,
  /** The "View post" permalink link inside a save-summary panel. */
  permalinkLink: '[data-test="permalink-link"]',
  /** Generic form error message. */
  error: ".error",
  /** Generic form submit button. */
  submit: 'button[type="submit"]',
  /** Logout link — present only once auth state is confirmed. */
  logoutLink: 'a[href="/logout"]',
  /** Login/register username field. */
  username: 'input[name="username"]',
  /** Login/register password field. */
  password: 'input[name="password"]',
  /** Reset-password new-password field. */
  newPassword: 'input[name="new_password"]',
  /** Top-bar page heading. */
  topbarHeading: ".j-topbar h1",
} as const;
