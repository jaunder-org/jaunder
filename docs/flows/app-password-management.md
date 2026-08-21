# App password management

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#app-password-management`

## Routes

- `route:/sessions`

## Endpoint census

| Endpoint                                     | Status  | Surface                                                                                                                                        |
| -------------------------------------------- | ------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `endpoint:/api/sessions/list`                | Covered | Reloads the session inventory after page load, app-password creation, and revoke actions.                                                      |
| `endpoint:/api/sessions/create_app_password` | Covered | Mints a labelled app password and returns the raw token once for copy/paste into external clients.                                             |
| `endpoint:/api/sessions/revoke`              | Covered | The browser-flow snapshot now covers own-session revocation; server integration also covers auth requirements and cross-user rejection (#707). |

`/sessions` is the authenticated user's app-password and session ledger. It
always renders the current session alongside externally minted app passwords so
the user can distinguish the browser session from long-lived editor credentials.

Creating an app password is a typed direct-bind action: the label must validate
before dispatch, and the returned raw token is shown once and never reloaded
from storage. The surrounding list resource keys off both create and revoke
versions, so the page re-reads after each mutation instead of patching
speculative local state.

Revocation stays user-scoped. The server confirms the target token hash belongs
to the authenticated account before deleting it, which is why the page can offer
one-button revoke controls without exposing a cross-account capability.
