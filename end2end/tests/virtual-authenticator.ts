import type { BrowserContext, CDPSession, Page } from "@playwright/test";

/**
 * A Chromium CDP authenticator for real WebAuthn ceremonies in browser tests.
 *
 * Credentials are resident and user-verified so `navigator.credentials.get()`
 * exercises the account-picker/discoverable-authentication path rather than a
 * test-provided credential response. The caller owns `dispose()`; authenticators
 * are process-scoped in Chrome and must never leak into a later test.
 */
export type VirtualAuthenticatorCredential = {
  credentialId: string;
  isResidentCredential: boolean;
};

export type VirtualAuthenticator = {
  credentials(): Promise<VirtualAuthenticatorCredential[]>;
  setCredentialSignCount(
    credential: VirtualAuthenticatorCredential,
    signCount: number,
  ): Promise<void>;
  dispose(): Promise<void>;
};

export async function installVirtualAuthenticator(
  context: BrowserContext,
  page: Page,
): Promise<VirtualAuthenticator> {
  const session = await context.newCDPSession(page);
  await session.send("WebAuthn.enable", { enableUI: false });
  const added = (await session.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  })) as unknown as { authenticatorId?: unknown };
  if (typeof added.authenticatorId !== "string") {
    throw new Error("CDP did not return a virtual authenticator ID");
  }
  const { authenticatorId } = added;

  return {
    credentials: () => credentialsFor(session, authenticatorId),
    setCredentialSignCount: (credential, signCount) =>
      setCredentialSignCount(session, authenticatorId, credential, signCount),
    dispose: () => disposeVirtualAuthenticator(session, authenticatorId),
  };
}

async function credentialsFor(
  session: CDPSession,
  authenticatorId: string,
): Promise<VirtualAuthenticatorCredential[]> {
  const result = (await session.send("WebAuthn.getCredentials", {
    authenticatorId,
  })) as unknown as { credentials?: unknown };
  if (
    !Array.isArray(result.credentials) ||
    !result.credentials.every(isVirtualAuthenticatorCredential)
  ) {
    throw new Error("CDP returned invalid virtual authenticator credentials");
  }
  return result.credentials;
}

async function setCredentialSignCount(
  session: CDPSession,
  authenticatorId: string,
  credential: VirtualAuthenticatorCredential,
  signCount: number,
): Promise<void> {
  // Chrome's current CDP accepts signCount, but Playwright's pinned protocol
  // declarations lag that field. Keep the escape at this audited boundary.
  await session.send(
    "WebAuthn.setCredentialProperties" as never,
    {
      authenticatorId,
      credentialId: credential.credentialId,
      signCount,
    } as never,
  );
}

function isVirtualAuthenticatorCredential(
  value: unknown,
): value is VirtualAuthenticatorCredential {
  return (
    typeof value === "object" &&
    value !== null &&
    "credentialId" in value &&
    typeof value.credentialId === "string" &&
    "isResidentCredential" in value &&
    typeof value.isResidentCredential === "boolean"
  );
}

/**
 * Cleanup is deliberately best-effort: a failed ceremony can close its target
 * before Playwright reaches this finally block, and teardown must not replace
 * that useful failure with a detached-CDP-session error.
 */
async function disposeVirtualAuthenticator(
  session: CDPSession,
  authenticatorId: string,
): Promise<void> {
  await session
    .send("WebAuthn.removeVirtualAuthenticator", { authenticatorId })
    .catch(() => undefined);
  await session.send("WebAuthn.disable").catch(() => undefined);
  await session.detach().catch(() => undefined);
}
