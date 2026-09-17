# Copy App Password

## Outcome

After minting an App Password on `/sessions`, the User can copy the one-time raw
credential with one explicit action instead of selecting it manually. The
credential remains visible so manual copying is still possible when browser
clipboard access is unavailable.

## Load-bearing decisions

- The one-time App Password presentation includes a button labelled **Copy app
  password** adjacent to the raw token.
- Copying is always user-initiated. Creating an App Password does not
  automatically write to the clipboard.
- The copy action writes only the raw App Password token, without its label,
  surrounding instructions, or whitespace.
- A successful write changes the control label to **Copied** for two seconds,
  then returns it to **Copy app password**.
- A rejected or unavailable clipboard write leaves the raw token visible and
  displays an inline error explaining that the App Password could not be copied.
- The same copy affordance is present for both confirmed creation and
  commit-indeterminate creation because either response contains the only
  display of the raw token.
- The existing one-time-secret contract remains unchanged: Jaunder stores only
  the token hash and never redisplays the raw App Password after this response.

## Acceptance

- Minting an App Password on `/sessions` shows its raw token and an adjacent
  **Copy app password** control.
- Activating the control writes exactly the displayed raw token to the browser
  clipboard.
- A successful write visibly acknowledges **Copied** for two seconds and then
  restores **Copy app password**.
- If the clipboard write fails, the page shows a user-visible error while the
  raw token remains available for manual selection.
- Confirmed and commit-indeterminate creation responses expose the same copy
  interaction.
- Automated browser coverage proves the successful copy flow, including the
  exact clipboard value and visible acknowledgement.
- Transient review artifacts provide a before/after screenshot pair at
  `/sessions`, 1280×720, default theme, authenticated, with a freshly minted App
  Password visible: the `origin/main` baseline and finished branch use the same
  viewport and UI state; the finished capture visibly includes the one-time
  warning, complete token, and copy control with no overlap or clipping.

## Boundaries

- This work does not change App Password generation, storage, authentication,
  revocation, labels, expiry, or transport semantics.
- It does not add automatic copying, token redisplay, clipboard history,
  download, QR-code, or password-manager integration.
- It does not redesign the Sessions list or distinguish App Password records
  from browser Sessions in storage.
- It introduces no new domain term or architectural decision; it reuses the
  existing App Password and browser clipboard contracts.
