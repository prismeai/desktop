/**
 * Prisme.ai desktop — connection screen.
 *
 * On load it resolves the startup state with Rust:
 *   - "app"     → a valid session was reused; Rust opens the app window.
 *   - "offline" → the stored server is unreachable; show a Retry screen.
 *   - "connect" → no stored session; show the sign-in form.
 */
import { invoke } from '@tauri-apps/api/core';

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

interface StartupState {
  mode: 'app' | 'offline' | 'connect';
  server: string | null;
}

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
  const loading = $<HTMLParagraphElement>('loading');
  const connect = $<HTMLDivElement>('connect');
  const offline = $<HTMLDivElement>('offline');
  const offlineText = $<HTMLParagraphElement>('offline-text');
  const retryBtn = $<HTMLButtonElement>('retry-btn');

  const form = $<HTMLFormElement>('connect-form');
  const input = $<HTMLInputElement>('server-url');
  const button = $<HTMLButtonElement>('connect-btn');
  const error = $<HTMLParagraphElement>('error');
  const hint = $<HTMLParagraphElement>('hint');

  let signing = false;

  const show = (el: HTMLElement): void => {
    loading.hidden = true;
    connect.hidden = true;
    offline.hidden = true;
    el.hidden = false;
  };

  const showError = (msg: string): void => {
    error.textContent = msg;
    error.hidden = false;
  };

  // --- Startup resolution (session reuse / offline) ---
  const resolve = async (): Promise<void> => {
    show(loading);
    let state: StartupState;
    try {
      state = await invoke<StartupState>('resolve_startup');
    } catch {
      state = { mode: 'connect', server: null };
    }
    if (state.mode === 'app') {
      // Rust is opening the app window and closing this one; keep loading.
      return;
    }
    if (state.mode === 'offline') {
      offlineText.textContent = `We couldn't reach ${state.server ?? 'your Prisme.ai server'}. Check your connection and try again.`;
      show(offline);
      return;
    }
    // connect
    void invoke<string | null>('get_server_url').then((prev) => {
      if (prev) input.value = prev;
    });
    show(connect);
    input.focus();
  };

  retryBtn.addEventListener('click', () => void resolve());

  // --- Sign-in ---
  const reset = (): void => {
    signing = false;
    input.disabled = false;
    button.disabled = false;
    button.textContent = 'Connect';
    hint.hidden = true;
  };

  const startSignIn = async (): Promise<void> => {
    error.hidden = true;
    const origin = normalizeServerUrl(input.value);
    if (!origin) {
      showError('Please enter a valid server URL.');
      return;
    }
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
      await invoke('open_app_window', { url: result.exchangeUrl });
    } catch (err) {
      reset();
      if (String(err) !== '__cancelled__') showError(String(err));
    }
  };

  form.addEventListener('submit', (e) => {
    e.preventDefault();
    if (signing) {
      void invoke('cancel_sign_in');
      return;
    }
    void startSignIn();
  });
  input.addEventListener('input', () => (error.hidden = true));

  void resolve();
});
