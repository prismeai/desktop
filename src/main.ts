/**
 * Prisme.ai desktop — connection screen logic.
 *
 * Runs in the trusted local window. On connect, it asks Rust to open the remote
 * server in its own window (native notifications + downloads) and close this one.
 */
import { invoke } from '@tauri-apps/api/core';

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

const normalizeServerUrl = (raw: string): string | null => {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const withScheme = /^https?:\/\//i.test(trimmed)
    ? trimmed
    : `https://${trimmed}`;
  try {
    const url = new URL(withScheme);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') return null;
    return url.origin;
  } catch {
    return null;
  }
};

window.addEventListener('DOMContentLoaded', () => {
  const form = $<HTMLFormElement>('connect-form');
  const input = $<HTMLInputElement>('server-url');
  const button = $<HTMLButtonElement>('connect-btn');
  const error = $<HTMLParagraphElement>('error');
  const hint = $<HTMLParagraphElement>('hint');

  let signing = false;

  const showError = (msg: string): void => {
    error.textContent = msg;
    error.hidden = false;
  };

  const reset = (): void => {
    signing = false;
    input.disabled = false;
    button.disabled = false;
    button.textContent = 'Connect';
    hint.hidden = true;
  };

  // Prefill with the last-used server.
  void invoke<string | null>('get_server_url').then((prev) => {
    if (prev) input.value = prev;
  });

  const startSignIn = async (): Promise<void> => {
    error.hidden = true;
    const origin = normalizeServerUrl(input.value);
    if (!origin) {
      showError('Please enter a valid server URL.');
      return;
    }
    // Enter signing state — the button becomes "Cancel" so the user is never
    // stuck if the browser handoff doesn't return (e.g. unregistered scheme).
    signing = true;
    input.disabled = true;
    button.textContent = 'Cancel';
    hint.hidden = false;
    try {
      await invoke('set_server_url', { url: origin });
      const result = await invoke<{ exchangeUrl: string }>('sign_in', {
        apiRoot: origin,
      });
      button.disabled = true;
      button.textContent = 'Opening…';
      hint.hidden = true;
      // The exchange URL sets the httpOnly session cookie in the webview and
      // redirects to the console — no token handled by the page.
      await invoke('open_app_window', { url: result.exchangeUrl });
    } catch (err) {
      reset();
      // Silent when the user just closed the sign-in sheet.
      if (String(err) !== '__cancelled__') showError(String(err));
    }
  };

  form.addEventListener('submit', (e) => {
    e.preventDefault();
    if (signing) {
      // Cancel: dropping the Rust-side sender rejects the pending sign_in,
      // which lands in the catch above and resets the UI.
      void invoke('cancel_sign_in');
      return;
    }
    void startSignIn();
  });
  input.addEventListener('input', () => (error.hidden = true));
});
