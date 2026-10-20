export type ConnectionState = 'checking' | 'connected' | 'not-paired' | 'auth-required' | 'unavailable' | 'storage-unavailable';

export function connectionFailure(error: unknown): { state: ConnectionState; message: string } {
  if (error && typeof error === 'object' && 'kind' in error && 'message' in error && typeof error.message === 'string') {
    const states = { NOT_PAIRED: 'not-paired', AUTH_REQUIRED: 'auth-required', UNAVAILABLE: 'unavailable', STORAGE_UNAVAILABLE: 'storage-unavailable' } as const;
    const state = states[error.kind as keyof typeof states];
    if (state) return { state, message: error.message };
  }
  return { state: 'unavailable', message: 'The connection could not be checked. Retry in a moment.' };
}

export const connectionCopy: Record<ConnectionState, { label: string; title: string; description: string; action: string }> = {
  checking: { label: 'Checking connection', title: 'Checking this PC', description: 'Checking your connection to rbgs.io.', action: 'Checking…' },
  connected: { label: 'Connected', title: 'Account connected', description: 'This PC can reach your rbgs.io account.', action: 'Connection settings' },
  'not-paired': { label: 'Not connected', title: 'Connect your account', description: 'Allow this PC to load your matches and upload their data. Confirm in your browser.', action: 'Connect account' },
  'auth-required': { label: 'Reconnect account', title: 'Reconnect your account', description: 'This PC no longer has access to your account. Connect again to continue. Your saved matches are still on this PC.', action: 'Reconnect account' },
  unavailable: { label: 'Connection unavailable', title: 'Cannot reach rbgs.io', description: 'Check your internet connection and try again. Your saved matches are still on this PC.', action: 'Retry connection' },
  'storage-unavailable': { label: 'Cannot read saved login', title: 'Cannot read your saved login', description: 'Windows could not open your saved login. Try again. See Connection details if this keeps happening.', action: 'Retry connection' },
};
