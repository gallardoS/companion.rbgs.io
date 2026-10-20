import { connectionCopy, type ConnectionState } from './connection';
import { Icon } from './Navigation';
import type { useSavedVariables } from './SavedVariables';
import { gameSetup, type SettingsTarget } from './gameSetup';
import { usePrivacy } from './Privacy';

type Props = {
  accountName: string | null;
  connection: ConnectionState;
  capturePermissions: boolean;
  connecting: boolean;
  working: boolean;
  checking: boolean;
  storageWarning: string | null;
  source: ReturnType<typeof useSavedVariables>;
  onSettings: (target?: SettingsTarget) => void;
  onConnect: () => void;
  onRetry: () => void;
};

export function Overview({ accountName, connection, capturePermissions, connecting, working, checking, storageWarning, source, onSettings, onConnect, onRetry }: Props) {
  const { view } = source;
  const game = view?.game;
  const setup = gameSetup(source);
  const privacy = usePrivacy();
  const installation = view?.installation ? privacy.wowPath(view.installation) : null;
  const savedAt = game?.fileModifiedAt != null ? new Date(game.fileModifiedAt * 1000) : null;
  let next: { title: string; description: string; action: string; run: () => void; disabled?: boolean; warning?: boolean } | null = null;

  if (connecting) next = { title: 'Waiting for browser approval', description: 'Approve this PC in your browser to connect automatically.', action: 'View connection', run: () => onSettings('account') };
  else if (connection !== 'connected') {
    const copy = connectionCopy[connection];
    next = { title: copy.title, description: copy.description, action: copy.action,
      run: connection === 'not-paired' || connection === 'auth-required' ? onConnect : onRetry,
      disabled: working || checking, warning: connection !== 'checking' };
  }
  else if (!capturePermissions) next = { title: 'Allow match uploads', description: 'Approve match access and uploads for this PC in your browser.', action: 'Allow access', run: onConnect, disabled: working, warning: true };
  else if (!setup.ready) next = { title: setup.title, description: setup.description,
    action: !view && !setup.readerFailure ? 'Checking…' : !setup.folderReady && setup.target === 'folder' && !setup.readerFailure ? 'Choose folder' : setup.target === 'wow-account' ? 'Choose account' : 'Open settings',
    run: !setup.folderReady && setup.target === 'folder' && view && !setup.readerFailure ? () => { void source.run('choose_forever_installation'); } : () => onSettings(setup.target),
    disabled: !view && !setup.readerFailure || source.busy, warning: setup.readerFailure || setup.target !== 'folder' && game?.status !== 'UNSTABLE_FILE' };

  return <>
    {next ? <section className={`next-action next-${next.warning ? 'warning' : 'pending'}`} aria-labelledby="next-action-title">
      <div className="next-action-copy"><h2 id="next-action-title">{next.title}</h2><p>{next.description}</p></div>
      <button className="button-primary" disabled={next.disabled} onClick={next.run}>{next.action}<Icon name="arrow" /></button>
    </section> : <div className="readiness-steps" role="group" aria-label="Companion status">
      <div className="readiness-step"><Icon name="account" /><strong>Account connected</strong><div className="readiness-copy">
        {accountName && <span className="readiness-value">{privacy.accountName(accountName)}</span>}
        <span className="readiness-meta">Match uploads allowed</span>
      </div></div>
      <div className="readiness-step"><Icon name="folder" /><strong>WoW folder selected</strong><div className="readiness-copy">
        {setup.selectedAccount && <span className="readiness-value">WoW account: {privacy.wowAccount(setup.selectedAccount.label)}</span>}
        {installation && <span className="readiness-path" title={installation}>{installation}</span>}
      </div></div>
      <div className="readiness-step"><Icon name="addon" /><strong>Addon detected</strong><div className="readiness-copy">
        {game?.characterName ? <><span className="readiness-value">{game.characterName}</span>{game.realm && <span className="readiness-meta">{game.realm}</span>}</> : <span className="readiness-meta">Ready for your first match</span>}
        {savedAt && <time className="readiness-meta readiness-saved" dateTime={savedAt.toISOString()} title={savedAt.toLocaleString()}>File saved {savedAt.toLocaleString([], { dateStyle: 'short', timeStyle: 'short' })}</time>}
      </div></div>
    </div>}

    {storageWarning && <p className="storage-warning" role="alert">Cannot save match data. {storageWarning}</p>}
  </>;
}
