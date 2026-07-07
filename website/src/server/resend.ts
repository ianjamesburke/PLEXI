import { readEnv } from './env';

const AUDIENCE_ID = 'e912b964-512d-40dc-a429-0aa2fb4c4c9e';
const FROM_ADDRESS = 'Plexi <auth@plexiapp.com>';

/** Thrown when RESEND_API_KEY is absent — fail fast, no silent fallback. */
export class MissingResendApiKeyError extends Error {
  constructor() {
    super('RESEND_API_KEY is not set — cannot send transactional email');
    this.name = 'MissingResendApiKeyError';
  }
}

export async function addSubscriber(email: string): Promise<void> {
  const apiKey = readEnv('RESEND_API_KEY');
  if (!apiKey) throw new MissingResendApiKeyError();

  const res = await fetch(`https://api.resend.com/audiences/${AUDIENCE_ID}/contacts`, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${apiKey}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ email, unsubscribed: false }),
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Resend API error ${res.status}: ${body}`);
  }
}

export type MagicLinkPurpose = 'login' | 'device_approve' | 'account_delete';

export interface OutboundEmail {
  to: string;
  subject: string;
  html: string;
  text: string;
}

/**
 * Low-level email transport. Overridable so tests capture the outbound email
 * (and the magic-link URL inside it) instead of hitting Resend. We own this
 * wrapper, so a test override here is legitimate — we never fake Resend's
 * own response shapes.
 */
export type EmailTransport = (email: OutboundEmail) => Promise<void>;

let transport: EmailTransport | null = null;

export function setEmailTransport(t: EmailTransport): void {
  transport = t;
}

export function resetEmailTransport(): void {
  transport = null;
}

const SUBJECTS: Record<MagicLinkPurpose, string> = {
  login: 'Sign in to Plexi',
  device_approve: 'Approve your Plexi sign-in',
  account_delete: 'Confirm deleting your Plexi account',
};

const INTROS: Record<MagicLinkPurpose, string> = {
  login: 'Click below to sign in to your Plexi account.',
  device_approve: 'Click below to approve the device that is trying to sign in.',
  account_delete:
    'Click below to permanently delete your Plexi account. This cannot be undone.',
};

async function resendTransport(email: OutboundEmail): Promise<void> {
  const apiKey = readEnv('RESEND_API_KEY');
  if (!apiKey) throw new MissingResendApiKeyError();

  const res = await fetch('https://api.resend.com/emails', {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${apiKey}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({
      from: FROM_ADDRESS,
      to: email.to,
      subject: email.subject,
      html: email.html,
      text: email.text,
    }),
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Resend API error ${res.status}: ${body}`);
  }
}

export async function sendMagicLink(
  email: string,
  url: string,
  purpose: MagicLinkPurpose,
): Promise<void> {
  const subject = SUBJECTS[purpose];
  const intro = INTROS[purpose];
  const html = `<div style="font-family:system-ui,sans-serif;line-height:1.5">
  <p>${intro}</p>
  <p><a href="${url}">${subject}</a></p>
  <p style="color:#888;font-size:13px">This link expires in 15 minutes and can be used once. If you didn't request it, ignore this email.</p>
</div>`;
  const text = `${intro}\n\n${url}\n\nThis link expires in 15 minutes and can be used once.`;

  const send = transport ?? resendTransport;
  await send({ to: email, subject, html, text });
}
