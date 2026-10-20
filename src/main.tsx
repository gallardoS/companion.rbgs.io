import { StrictMode, useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { createRoot } from 'react-dom/client';
import { SavedVariables, useSavedVariables } from './SavedVariables';
import { MatchHistory, useMatchHistory } from './MatchHistory';
import { CaptureAssignments, type AssignmentSummary } from './CaptureAssignments';
import { ConnectionRetry } from './ConnectionRetry';
import { connectionCopy, connectionFailure, type ConnectionState } from './connection';
import { Overview } from './Overview';
import { PrivacyProvider, PrivacySettings } from './Privacy';
import { gameSetup, type SettingsTarget } from './gameSetup';
import { errorDetail, Feedback } from './Feedback';
import { useAssignmentActivity } from './useAssignmentActivity';
import { Icon, Navigation, tabs, type TabId } from './Navigation';
import './styles.css';

function App() {
  const [activeTab, setActiveTab] = useState<TabId>('home');
  const [uploadSettingsTarget, setUploadSettingsTarget] = useState<HTMLDivElement | null>(null);
  const currentTab = useRef<TabId>('home');
  const tabScroll = useRef<Record<TabId, number>>({ home: 0, history: 0, settings: 0 });
  const [connection, setConnection] = useState<ConnectionState>('checking');
  const [checking, setChecking] = useState(true);
  const content = useRef<HTMLElement>(null);
  const [pairing, setPairing] = useState<{ code: string; expiresAt: string; approvalUrl: string } | null>(null);
  const [capturePermissions, setCapturePermissions] = useState(false);
  const [accountId, setAccountId] = useState('');
  const [accountName, setAccountName] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [webConnecting, setWebConnecting] = useState(false);
  const webActive = useRef(false);
  const working = busy || webConnecting;
  const connected = connection === 'connected';
  const connecting = !!pairing || webConnecting;
  const accountReady = connected && capturePermissions && !connecting;
  const activity = useAssignmentActivity(accountId, connected && capturePermissions && !connecting);
  const [focusCaptureId, setFocusCaptureId] = useState<string | null>(null);
  const [noticeError, setNoticeError] = useState('');
  const [openingMatch, setOpeningMatch] = useState(false);
  const openRequest = useRef(0);
  const currentAccount = useRef(accountId);
  currentAccount.current = accountId;
  const lastOpen = useRef('');
  const [message, setMessage] = useState('');
  const [pairingError, setPairingError] = useState<string | null>(null);
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [storageWarning, setStorageWarning] = useState<string | null>(null);
  const [assignmentSummary, setAssignmentSummary] = useState<(AssignmentSummary & { accountId: string }) | null>(null);
  const inFlight = useRef(false);
  const checkEpoch = useRef(0);
  const source = useSavedVariables();
  const historyScope = source.view?.selectedSource && !connecting
    ? JSON.stringify([source.view.installation, source.view.selectedSource, accountId]) : null;
  const history = useMatchHistory(historyScope);
  const onAssignmentSummary = useCallback((summary: AssignmentSummary) => setAssignmentSummary({ ...summary, accountId }), [accountId]);

  async function check() {
    const epoch = ++checkEpoch.current;
    setChecking(true);
    try {
      const status = await invoke<{ accountId: string; accountName?: string | null; localStorageWarning?: string | null; capturePermissions: boolean }>('connection_status');
      if (epoch !== checkEpoch.current) return;
      setConnection('connected');
      setAccountId(status.accountId ?? '');
      setAccountName(typeof status.accountName === 'string' && status.accountName.trim() ? status.accountName : null);
      setCapturePermissions(status.capturePermissions === true);
      setConnectionError(null);
      setStorageWarning(status.localStorageWarning ?? null);
      setMessage('');
    } catch (error) {
      if (epoch !== checkEpoch.current) return;
      const failure = connectionFailure(error);
      setConnection(failure.state);
      setConnectionError(failure.message);
      if (failure.state === 'not-paired' || failure.state === 'auth-required' || failure.state === 'storage-unavailable') {
        setAccountId('');
        setAccountName(null);
        setCapturePermissions(false);
        setStorageWarning(null);
      }
    } finally {
      if (epoch === checkEpoch.current) setChecking(false);
    }
  }

  useEffect(() => {
    const refresh = () => { if (!pairing && !webActive.current) void check(); };
    refresh();
    const timer = window.setInterval(refresh, 15000);
    window.addEventListener('focus', refresh);
    return () => { ++checkEpoch.current; window.clearInterval(timer); window.removeEventListener('focus', refresh); };
  }, [pairing]);

  useEffect(() => {
    let disposed = false;
    let last = '';
    let inProgress = false;
    const read = async () => {
      if (inProgress) return;
      inProgress = true;
      try {
        const progress = await invoke<{ status: string; message: string | null }>('web_connection_state');
        const signature = JSON.stringify(progress);
        if (disposed || signature === last) return;
        last = signature;
        const pending = progress.status === 'CONNECTING';
        webActive.current = pending;
        setWebConnecting(pending);
        if (pending) {
          setPairingError(null);
          ++checkEpoch.current;
          setChecking(false);
          setPairing(null);
          setAccountId('');
          setAccountName(null);
          setCapturePermissions(false);
          setConnection('checking');
          setConnectionError(null);
          setMessage(progress.message ?? 'Connecting from rbgs.io…');
        } else if (progress.status === 'CONNECTED') {
          setPairingError(null);
          setPairing(null);
          void check();
        } else if (progress.status === 'ERROR') {
          setPairingError(progress.message ?? 'Connection could not be completed.');
          void check();
        }
      } catch {}
      finally { inProgress = false; }
    };
    void read();
    const timer = window.setInterval(() => void read(), 1000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, []);

  useEffect(() => {
    if (!pairing) return;
    let disposed = false;
    const poll = async () => {
      if (inFlight.current) return;
      inFlight.current = true;
      try {
        const status = await invoke<string>('poll_pairing');
        if (!disposed) {
          setPairingError(null);
          if (status === 'PAIRED') { setPairing(null); await check(); }
        }
      } catch (error) { if (!disposed) setPairingError(errorDetail(error)); }
      finally { inFlight.current = false; }
    };
    const timer = window.setInterval(() => void poll(), 5000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [pairing]);

  function changeTab(tab: TabId) {
    tabScroll.current[currentTab.current] = content.current?.scrollTop ?? 0;
    currentTab.current = tab;
    setActiveTab(tab);
    document.getElementById(`tab-${tab}`)?.focus();
    requestAnimationFrame(() => {
      if (currentTab.current === tab) content.current?.scrollTo(0, tabScroll.current[tab]);
    });
  }

  function openSettings(target: SettingsTarget = 'account') {
    changeTab('settings');
    requestAnimationFrame(() => {
      const field = document.getElementById(`settings-${target}`);
      field?.focus({ preventScroll: true });
      field?.scrollIntoView({ block: 'nearest' });
    });
  }

  useEffect(() => {
    ++openRequest.current;
    setOpeningMatch(false);
    setFocusCaptureId(null);
    setNoticeError('');
    lastOpen.current = '';
  }, [accountId]);

  useEffect(() => {
    const signature = `${accountId}/${activity?.openSequence ?? 0}`;
    if (activity?.openCaptureId && signature !== lastOpen.current) {
      lastOpen.current = signature;
      setFocusCaptureId(activity.openCaptureId);
      changeTab('home');
    }
  }, [accountId, activity?.openCaptureId, activity?.openSequence]);
  const readyCaptureId = activity?.captureIds.find(id => !history.view?.matches.some(match => match.captureId === id));
  async function openTestMatch() {
    if (!readyCaptureId || openingMatch) return;
    const request = ++openRequest.current;
    const requestAccount = accountId;
    setOpeningMatch(true); setNoticeError('');
    try {
      await invoke('open_test_match', { captureId: readyCaptureId });
      if (request !== openRequest.current || requestAccount !== currentAccount.current) return;
      setFocusCaptureId(readyCaptureId); changeTab('home');
    }
    catch { if (request === openRequest.current && requestAccount === currentAccount.current) setNoticeError('This test match could not be opened. Check your connection and try again.'); }
    finally { if (request === openRequest.current) setOpeningMatch(false); }
  }

  async function start() {
    if (working) return;
    changeTab('settings');
    ++checkEpoch.current;
    setBusy(true);
    setPairingError(null);
    setMessage('Preparing your connection…');
    try {
      const result = await invoke<{ code: string; expiresAt: string; approvalUrl: string }>('start_pairing', { capturePermissions: true });
      setPairing(result);
      setConnectionError(null);
      setMessage('');
      try { await invoke('open_pairing_browser'); }
      catch { setMessage('Click Approve in browser to continue. If it does not open, use the code below.'); }
    } catch (error) { setMessage(''); setPairingError(errorDetail(error)); }
    finally { setBusy(false); setChecking(false); }
  }

  function openUploads() {
    openSettings('uploads');
  }

  const selected = tabs.find(tab => tab.id === activeTab)!;
  const copy = connectionCopy[connection];
  const canPair = connection === 'not-paired' || connection === 'auth-required';
  const retryConnection = () => void check();
  const connectionAction = canPair ? () => void start() : retryConnection;
  const queue = history.view?.queue;
  const companionReady = accountReady && gameSetup(source).ready && !storageWarning && !history.error
    && (!queue || queue.status === 'READY' && queue.conflicts === 0 && queue.needsAuth === 0 && queue.waitingAssignment === 0 && (queue.blocked ?? 0) === 0);
  return <div className="app-shell">
    <header className="app-header"><div className="header-inner">
      <div className="brand-lockup"><span className="brand">rbgs.io</span><span className="product-name">Companion</span></div>
      <div className="connection-status"><span className={`connection-badge ${connecting ? 'is-pending' : connected ? 'is-connected' : connection === 'checking' ? 'is-pending' : 'is-unavailable'}`} role="status"><span className="status-dot" />{webConnecting ? 'Connecting…' : pairing ? 'Waiting for approval' : copy.label}</span>
        {connectionError && !connecting && <ConnectionRetry checking={checking} disabled={working} onRetry={retryConnection} />}
      </div>
    </div></header>
    <Navigation active={activeTab} onChange={changeTab} />
    <main className="page-scroll" ref={content}>
      <div className="content">
        <header className="page-heading"><h1>{selected.title}</h1></header>
        {readyCaptureId && activeTab !== 'home' && <aside className="test-ready-notice" aria-label="Test match ready" role="status"><div><strong>Test match ready</strong><p>Get your match command before entering the battleground.</p>{noticeError && <p>{noticeError}</p>}</div><button className="button-primary" disabled={openingMatch} onClick={() => void openTestMatch()}>{openingMatch ? 'Opening…' : 'View test match'}<Icon name="arrow" /></button></aside>}
        <section id="panel-home" role="tabpanel" aria-labelledby="tab-home" hidden={activeTab !== 'home'} tabIndex={0}>
          {!!accountId && capturePermissions && <CaptureAssignments key={accountId} accountId={accountId} enabled={connected && !connecting} companionReady={companionReady} history={history.view} onSummary={onAssignmentSummary} uploadSettingsTarget={uploadSettingsTarget} activityVersion={activity?.version ?? 0} focusCaptureId={focusCaptureId} />}
          <Overview accountName={accountName} connection={connection} capturePermissions={capturePermissions} connecting={connecting} working={working} checking={checking} storageWarning={storageWarning} source={source} onSettings={openSettings} onConnect={() => void start()} onRetry={retryConnection} />

        </section>
        <section id="panel-history" className="history-page" role="tabpanel" aria-labelledby="tab-history" hidden={activeTab !== 'history'} tabIndex={0}>
          {!connecting && (!!source.view?.selectedSource || assignmentSummary?.accountId === accountId && assignmentSummary.closedTests.length > 0) ? <MatchHistory key={`history:${historyScope ?? accountId}`} accountId={accountId} sourceSelected={!!source.view?.selectedSource} tests={assignmentSummary?.accountId === accountId ? assignmentSummary.closedTests : []} history={history} onSettings={openSettings} onUploads={openUploads} /> : <div className="compact-empty"><h2>{connecting ? 'Connecting your account' : 'Set up your game data'}</h2><p>{connecting ? 'Your history will be available when the connection completes.' : 'Choose your WoW folder and account to load saved matches.'}</p><button onClick={() => openSettings(connecting ? 'account' : 'folder')}>Open settings<Icon name="arrow" /></button></div>}
        </section>
        <section id="panel-settings" role="tabpanel" aria-labelledby="tab-settings" hidden={activeTab !== 'settings'} tabIndex={0}>
          {storageWarning && <p className="storage-warning" role="alert">{storageWarning}</p>}
          <section className="settings-section" aria-labelledby="settings-account">
            <div className="section-heading"><h2 id="settings-account" tabIndex={-1}>rbgs.io account</h2></div>
            <div className="section-body">
              {accountReady ? <><div className="settings-state is-complete"><Icon name="check" /><strong>Account connected</strong></div><p className="settings-value">Match uploads allowed</p></> : <div className="connection-notice"><span className="notice-icon"><Icon name="account" /></span><div><strong>{connecting ? 'Waiting for browser approval' : connected ? 'Allow match uploads' : copy.title}</strong><p>{connecting ? 'The companion will connect automatically after approval.' : connected ? 'Approve match access and uploads for this PC in your browser.' : copy.description}</p></div></div>}
              {!connected && !connecting && <button className="button-primary" disabled={working || checking} onClick={connectionAction}>{checking ? 'Checking…' : copy.action}<Icon name={canPair ? 'arrow' : 'reload'} /></button>}
              {connected && !connecting && !capturePermissions && <button className="button-primary" disabled={working} onClick={() => void start()}>Allow match uploads<Icon name="arrow" /></button>}
              {connectionError && !connecting && <details className="diagnostics"><summary>Connection details</summary><p>{connectionError}</p></details>}
              {message && <p className="status-message" role="status">{message}</p>}
              {pairingError !== null && <Feedback title="Connection could not be completed" detail={pairingError}>{pairing ? 'Check the browser approval. If the request expired, start a new request below.' : 'Check your connection and try connecting again.'}</Feedback>}
              {pairing && <section className="pairing-panel" aria-label="Pairing">
                <button className="button-primary" disabled={busy} onClick={() => void invoke('open_pairing_browser').then(() => setMessage('')).catch(() => setMessage('The browser could not be opened. Try again or connect with the code below.'))}>Approve in browser<Icon name="arrow" /></button>
                <details><summary>Connect with a code instead</summary><output className="pairing-code">{pairing.code}</output><p className="field-hint">Approve only the code shown in your own companion.</p></details>
                <p className="field-hint">Expires at {new Date(pairing.expiresAt).toLocaleTimeString()}.</p>
                <details><summary>Connection not completing?</summary><p>Check that you approved the request in your browser. If it expired, start again.</p><button disabled={busy} onClick={() => void start()}>Start new request</button></details>
              </section>}
            </div>
          </section>
          <PrivacySettings />
          <section className="settings-section" aria-labelledby="settings-uploads">
            <div className="section-heading"><h2 id="settings-uploads" tabIndex={-1}>Upload options</h2></div>
            <div className="section-body"><div ref={setUploadSettingsTarget} />{(!accountId || !capturePermissions) && <p>Connect your account and allow match uploads to manage upload options.</p>}</div>
          </section>
          <SavedVariables source={source} />
          <details className="settings-help background-help"><summary>Running in the background</summary><p>The companion keeps running when you close this window. To stop it, right-click its icon next to the Windows clock and choose <strong>Exit</strong>.</p></details>
        </section>
      </div>
    </main>
  </div>;
}

const root = createRoot(document.getElementById('root')!);
root.render(<StrictMode><PrivacyProvider><App /></PrivacyProvider></StrictMode>);
if (import.meta.hot) import.meta.hot.dispose(() => root.unmount());
