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

  const showError = (msg: string): void => {
    error.textContent = msg;
    error.hidden = false;
  };

  // Prefill with the last-used server.
  void invoke<string | null>('get_server_url').then((prev) => {
    if (prev) input.value = prev;
  });

  const connect = async (): Promise<void> => {
    error.hidden = true;
    const origin = normalizeServerUrl(input.value);
    if (!origin) {
      showError('Please enter a valid server URL.');
      return;
    }
    button.disabled = true;
    button.textContent = 'Signing in…';
    try {
      await invoke('set_server_url', { url: origin });
      // Native OIDC in the system browser; resolves once the callback returns.
      const result = await invoke<{ accessToken: string; consoleUrl: string }>(
        'sign_in',
        { apiRoot: origin }
      );
      button.textContent = 'Opening…';
      await invoke('open_app_window', {
        url: result.consoleUrl,
        token: result.accessToken,
      });
    } catch (err) {
      button.disabled = false;
      button.textContent = 'Connect';
      showError(String(err));
    }
  };

  form.addEventListener('submit', (e) => {
    e.preventDefault();
    void connect();
  });
  input.addEventListener('input', () => (error.hidden = true));
});
