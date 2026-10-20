import { StrictMode, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { createRoot } from 'react-dom/client';
import { SavedVariables } from './SavedVariables';
import { CaptureAssignments } from './CaptureAssignments';
import { ConnectionRetry } from './ConnectionRetry';
import { connectionCopy, connectionFailure, type ConnectionState } from './connection';
import { Icon, Navigation, tabs, type TabId } from './Navigation';
import './styles.css';

function App() {
  const [activeTab, setActiveTab] = useState<TabId>('account');
  const [checking, setChecking] = useState(true);
  const content = useRef<HTMLElement>(null);
  const [pairing, setPairing] = useState<{ code: string; expiresAt: string; approvalUrl: string } | null>(null);
  const [connection, setConnection] = useState<ConnectionState>('checking');
  const connected = connection === 'connected';
  const [capturePermissions, setCapturePermissions] = useState(false);
  const [accountId, setAccountId] = useState('');
  const [busy, setBusy] = useState(false);
  const [webConnecting, setWebConnecting] = useState(false);
  const webActive=useRef(false);
  const working=busy||webConnecting;
  const [message, setMessage] = useState('Checking saved connection…');
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [storageWarning, setStorageWarning] = useState<string | null>(null);
  const inFlight = useRef(false);
  const checkEpoch = useRef(0);
  async function check() {
    const epoch = ++checkEpoch.current;
    setChecking(true);
    try {
      const status = await invoke<{ accountId: string; accountName?: string | null; localStorageWarning?: string | null; capturePermissions: boolean }>('connection_status');
      if (epoch !== checkEpoch.current) return;
      setConnection('connected');
      setAccountId(status.accountId ?? '');
      setCapturePermissions(status.capturePermissions === true);
      setConnectionError(null);
      setStorageWarning(status.localStorageWarning ?? null);
      setMessage('');
    } catch (error) {
      if (epoch !== checkEpoch.current) return;
      const failure = connectionFailure(error);
      setConnection(failure.state);
      setConnectionError(failure.message);
      setMessage(connectionCopy[failure.state].description);
      if (failure.state === 'not-paired' || failure.state === 'auth-required' || failure.state === 'storage-unavailable') {
        setAccountId('');
        setCapturePermissions(false);
        setStorageWarning(null);
      }
    } finally {
      if (epoch === checkEpoch.current) setChecking(false);
    }
  }

  useEffect(() => { const refresh=()=>{if(!pairing&&!webActive.current)void check();};refresh();const timer=window.setInterval(refresh,15000);window.addEventListener('focus',refresh);return()=>{++checkEpoch.current;window.clearInterval(timer);window.removeEventListener('focus',refresh);}; }, [pairing]);
  useEffect(()=>{
    let disposed=false;let last='';let inProgress=false;
    const read=async()=>{
      if(inProgress)return;inProgress=true;
      try {
        const progress=await invoke<{status:string;message:string|null}>('web_connection_state');
        const signature=JSON.stringify(progress);
        if(disposed||signature===last)return;last=signature;
        const pending=progress.status==='CONNECTING';webActive.current=pending;setWebConnecting(pending);
        if(pending){++checkEpoch.current;setChecking(false);setPairing(null);setAccountId('');setCapturePermissions(false);setConnection('checking');setConnectionError(null);setMessage(progress.message??'Connecting from rbgs.io…');}
        else if(progress.status==='CONNECTED'){setPairing(null);void check();}
        else if(progress.status==='ERROR'){setConnectionError(progress.message);setMessage(progress.message??'Connection could not be completed.');}
      }catch{/* The normal connection check remains available. */}finally{inProgress=false;}
    };
    void read();const timer=window.setInterval(()=>void read(),1000);
    return()=>{disposed=true;window.clearInterval(timer);};
  },[]);
  useEffect(() => {
    if (!pairing) return;
    let disposed = false;
    const poll = async () => {
      if (inFlight.current) return;
      inFlight.current = true;
      try {
        const status = await invoke<string>('poll_pairing');
        if (disposed) return;
        if (status === 'PAIRED') { setPairing(null); await check(); }
      } catch (error) { if (!disposed) setMessage(String(error)); }
      finally { inFlight.current = false; }
    };
    const timer = window.setInterval(() => void poll(), 5000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [pairing]);
  async function start(captures = true) {
    if (working) return;
    setBusy(true);
    try {
      const result = await invoke<{ code: string; expiresAt: string; approvalUrl: string }>('start_pairing', { capturePermissions: captures });
      setPairing(result); setConnectionError(null); setMessage('Waiting for your approval in web Settings.');
      try { await invoke('open_pairing_browser'); } catch { setMessage('Connection request created. Use Approve in browser, or the manual code option if your browser cannot open.'); }
    } catch (error) { setMessage(String(error)); }
    finally { setBusy(false); }
  }
  function changeTab(tab: TabId) {
    setActiveTab(tab);
    content.current?.scrollTo(0, 0);
    document.getElementById(`tab-${tab}`)?.focus();
  }
  const selected = tabs.find(tab => tab.id === activeTab)!;
  const connectionLabel = webConnecting ? 'Connecting…' : pairing ? 'Awaiting approval' : connectionCopy[connection].label;
  return <div className="app-shell">
    <header className="app-header"><div className="header-inner">
      <div className="brand-lockup"><span className="brand">rbgs.io</span><span className="product-name">Desktop app</span></div>
      <div className="connection-status"><span className={`connection-badge ${pairing || webConnecting ? 'is-pending' : connected ? 'is-connected' : checking ? 'is-pending' : ''}`} role="status"><span className="status-dot" />{connectionLabel}</span>
        {connectionError && !pairing && <ConnectionRetry checking={checking} disabled={working} onRetry={() => void check()} />}
      </div>
    </div></header>
    <Navigation active={activeTab} onChange={changeTab} />
    <main className="page-scroll" ref={content}>
      <div className="content">
        <header className="page-heading"><p className="eyebrow">WORLD OF WARCRAFT: FOREVER</p><h1>{selected.title}</h1><p>{selected.description}</p></header>
        <section id="panel-account" role="tabpanel" aria-labelledby="tab-account" hidden={activeTab !== 'account'} tabIndex={0}>
          {storageWarning && <p className="storage-warning" role="alert">{storageWarning}</p>}
          {(!connected || pairing || message) && <section className="settings-section" aria-labelledby="connection-heading">
            <div className="section-heading"><h2 id="connection-heading">rbgs.io account</h2></div>
            <div className="section-body">
              <div className="connection-notice"><span className="notice-icon"><Icon name="account" /></span><div><strong>{pairing ? 'Approve in your browser' : checking ? 'Checking this PC' : connected ? 'Account action required' : connectionCopy[connection].title}</strong><p role={connectionError ? 'alert' : 'status'}>{message}</p></div></div>
              {!connected && !webConnecting && <div className="device-form">
                <button className="button-primary" disabled={working || checking} onClick={() => void (pairing || connection === 'not-paired' || connection === 'auth-required' ? start() : check())}>{busy ? 'Creating request…' : pairing ? 'Generate a new code' : connectionCopy[connection].action}<Icon name="arrow" /></button></div>}
              {pairing && <section className="pairing-panel" aria-label="Pairing"><h3>Finish connecting</h3><p>Approve this PC and its requested permissions in your browser. Connection finishes automatically.</p>
                <button className="button-primary" disabled={busy} onClick={()=>void invoke('open_pairing_browser').catch(error=>setMessage(String(error)))}>Approve in browser<Icon name="arrow" /></button>
                <details><summary>Use a code manually</summary><output className="pairing-code">{pairing.code}</output></details><p className="field-hint">Expires at {new Date(pairing.expiresAt).toLocaleTimeString()}.</p>
                <button disabled={busy} onClick={() => void invoke<string>('poll_pairing').then(status => { if (status === 'PAIRED') { setPairing(null); void check(); } else { setMessage('Waiting for your approval in web Settings.'); } }).catch(error =>setMessage(String(error)))}>Check approval</button>
              </section>}
              {!connected && <p className="field-hint">Approve only the code shown in your own companion.</p>}
            </div>
          </section>}
          <section className="settings-section" aria-labelledby="permissions-heading"><div className="section-heading"><h2 id="permissions-heading">Capture permissions</h2><p>Allow the companion to read your assignments and upload saved captures.</p></div><div className="section-body">
            <span className={`permission-label ${connected && capturePermissions ? 'is-enabled' : ''}`}><Icon name={connected && capturePermissions ? 'check' : 'captures'} />{connected && capturePermissions ? 'Capture access enabled' : 'Approval required'}</span>
            {connected && !capturePermissions && <button className="button-primary" disabled={working} onClick={() => void start(true)}>Enable capture permissions</button>}
            <p className="field-hint">{!connected ? 'Connect your account to approve capture access.' : capturePermissions ? 'Assignments and saved-capture uploads are available in the Captures tab.' : 'Approve assignment access and saved-capture uploads in web Settings.'}</p>
          </div></section>
        </section>
        <SavedVariables activeTab={activeTab} onTabChange={changeTab} />
        <section id="panel-captures" role="tabpanel" aria-labelledby="tab-captures" hidden={activeTab !== 'captures'} tabIndex={0}>
          {connected && capturePermissions && !pairing && !webConnecting ? <CaptureAssignments key={accountId} /> : <div className="empty-state"><span className="empty-icon"><Icon name="captures" /></span><h2>{connected ? 'Enable capture access' : 'Connect to get started'}</h2><p>{connected ? 'Approve capture permissions to see your test assignments and sync saved captures.' : 'Connect your rbgs.io account to see your test assignments and sync saved captures.'}</p><button className="button-primary" onClick={() => changeTab('account')}>Go to account<Icon name="arrow" /></button></div>}
        </section>
      </div>
      <footer className="app-footer"><span>Windows companion</span><span className="footer-game">World of Warcraft: Forever</span><span className="brand">rbgs.io</span></footer>
    </main>
  </div>;
}

createRoot(document.getElementById('root')!).render(<StrictMode><App /></StrictMode>);
