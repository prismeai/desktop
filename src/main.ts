/**
 * Prisme.ai desktop shell (Tauri POC) — setup screen logic.
 *
 * Runs in the trusted local "main" window. Talks to Rust via `invoke` and uses
 * the dialog plugin for native file pickers. On connect, it opens a SECOND
 * window pointed at the remote server URL and closes this one. That remote
 * window has no capability entry, so remote content cannot call any command.
 */
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';

const $ = <T extends HTMLElement>(id: string): T =>
  document.getElementById(id) as T;

interface FileInfo {
  name: string;
  bytes: number;
  path: string;
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
  const form = $<HTMLFormElement>('connect-form');
  const input = $<HTMLInputElement>('server-url');
  const button = $<HTMLButtonElement>('connect-btn');
  const error = $<HTMLParagraphElement>('error');
  const fsMsg = $<HTMLParagraphElement>('fs-msg');
  const readBtn = $<HTMLButtonElement>('read-btn');
  const writeBtn = $<HTMLButtonElement>('write-btn');

  const showError = (msg: string): void => {
    error.textContent = msg;
    error.hidden = false;
  };
  const showFs = (msg: string): void => {
    fsMsg.textContent = msg;
    fsMsg.hidden = false;
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
    button.textContent = 'Connecting…';
    try {
      await invoke('set_server_url', { url: origin });
      // Rust opens the remote app window (native notifications + downloads) and
      // closes this setup window.
      await invoke('open_app_window', { url: origin });
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

  // --- Local filesystem demo (desktop-only) ---
  readBtn.addEventListener('click', () => {
    void (async () => {
      const picked = await open({ multiple: false, directory: false });
      if (typeof picked !== 'string') return;
      try {
        const info = await invoke<FileInfo>('read_file_info', { path: picked });
        showFs(`Read "${info.name}" — ${info.bytes.toLocaleString()} bytes.`);
      } catch (err) {
        showFs(`Read failed: ${String(err)}`);
      }
    })();
  });

  writeBtn.addEventListener('click', () => {
    void (async () => {
      const target = await save({ defaultPath: 'prisme-desktop-test.txt' });
      if (!target) return;
      try {
        await invoke('write_text_file', {
          path: target,
          contents: 'Hello from the Prisme.ai desktop shell.\n',
        });
        showFs(`Wrote file to ${target}`);
      } catch (err) {
        showFs(`Write failed: ${String(err)}`);
      }
    })();
  });
});
