/**
 * Named selector constants for the e2e suite (#263).
 *
 * The same high-frequency CSS selector strings were literaled across many spec
 * files, so a markup rename or a typo touched N files with no compiler help.
 * Route those through `SEL` for a single source of truth and a uniform quote
 * style. One-off / rarely-repeated selectors stay inline at their call sites.
 */

export const SEL = {
  /** Save-summary panel shown after a successful compose/publish. */
  saveSummary: ".j-save-summary",
  /** Post-composer body textarea. */
  postBody: 'textarea[name="body"]',
  /** Publish/unpublish submit button; `value` is the boolean string. */
  publishButton: (value: string) => `button[name="publish"][value="${value}"]`,
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
