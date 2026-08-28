import { useCallback, useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./App.css";

type Session = { id: string; name: string; folder: string; createdAt: number; count: number; bytes: number; error: string | null };
type Screenshot = { name: string; bytes: number; modifiedAt: number };
type Snapshot = {
  sourceDir: string | null; storageDir: string | null; activeSessionId: string | null;
  monitoring: boolean; pendingCount: number; sessions: Session[];
  screenshots: Screenshot[]; lastError: string | null;
};
const empty: Snapshot = { sourceDir: null, storageDir: null, activeSessionId: null, monitoring: false, pendingCount: 0, sessions: [], screenshots: [], lastError: null };
const desktop = isTauri();

function bytes(value: number) {
  if (value < 1024) return `${value} B`;
  const exponent = Math.min(Math.floor(Math.log(value) / Math.log(1024)), 3);
  return `${(value / 1024 ** exponent).toFixed(exponent > 1 ? 1 : 0)} ${["B", "KB", "MB", "GB"][exponent]}`;
}
function displayPath(path: string | null) {
  return path?.replace(/^\\\\\?\\UNC\\/, "\\\\").replace(/^\\\\\?\\/, "") ?? "Not selected";
}
function Icon({ kind, size = 20 }: { kind: "folder" | "image" | "plus" | "play" | "pause" | "settings" | "arrow" | "check" | "close"; size?: number }) {
  const paths = {
    folder: <path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v10H3V7Z" />,
    image: <><rect x="3" y="3" width="18" height="18" rx="3" /><circle cx="8" cy="8" r="1.5" /><path d="m3 17 5-5 4 4 3-3 6 6" /></>,
    plus: <path d="M12 5v14M5 12h14" />,
    play: <path d="m8 5 11 7-11 7V5Z" />,
    pause: <path d="M8 5v14M16 5v14" />,
    settings: <><path d="M4 7h16M4 17h16" /><circle cx="9" cy="7" r="3" /><circle cx="15" cy="17" r="3" /></>,
    arrow: <path d="M7 17 17 7M7 7h10v10" />,
    check: <path d="m5 12 4 4L19 6" />,
    close: <path d="m6 6 12 12M6 18 18 6" />,
  };
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths[kind]}</svg>;
}

function App() {
  const [data, setData] = useState<Snapshot>(empty);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(desktop);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modal, setModal] = useState<"folders" | "session" | null>(null);
  const [source, setSource] = useState("");
  const [destination, setDestination] = useState("");
  const [name, setName] = useState("");
  const [filter, setFilter] = useState("");
  const dialog = useRef<HTMLDialogElement>(null);
  const selectedRef = useRef<string | null>(null);
  const requestRef = useRef(0);

  const refresh = useCallback(async () => {
    if (!desktop) return;
    const request = ++requestRef.current;
    const selection = selectedRef.current;
    try {
      const next = await invoke<Snapshot>("get_state", { selectedId: selection });
      if (request !== requestRef.current || selection !== selectedRef.current) return;
      setData(next);
      if (!selection && next.sessions.length) {
        const id = next.activeSessionId ?? next.sessions[0].id;
        selectedRef.current = id;
        setSelectedId(id);
      }
    } catch (e) { setError(String(e)); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 2000);
    return () => window.clearInterval(timer);
  }, [refresh]);
  useEffect(() => { void refresh(); }, [selectedId, refresh]);
  useEffect(() => {
    if (modal) {
      dialog.current?.showModal();
      dialog.current?.querySelector<HTMLInputElement>(modal === "session" ? "#session-name" : "#source-folder")?.focus();
    }
    else dialog.current?.close();
  }, [modal]);

  function select(id: string) {
    selectedRef.current = id;
    setSelectedId(id);
    setData(current => ({ ...current, screenshots: [] }));
    setFilter("");
  }
  async function perform(action: () => Promise<void>) {
    if (busy || !desktop) return;
    setBusy(true); setError(null); setNotice(null);
    try { await action(); await refresh(); }
    catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }
  function folderSetup() {
    setError(null);
    setSource(data.sourceDir ? displayPath(data.sourceDir) : "");
    setDestination(data.storageDir ? displayPath(data.storageDir) : "");
    setModal("folders");
  }
  async function browse(target: "source" | "destination") {
    await perform(async () => {
      const folder = await invoke<string | null>("choose_folder");
      if (folder) (target === "source" ? setSource : setDestination)(displayPath(folder));
    });
  }

  const selected = data.sessions.find(session => session.id === selectedId);
  const active = data.sessions.find(session => session.id === data.activeSessionId);
  const configured = !!data.sourceDir && !!data.storageDir;
  const files = data.screenshots.filter(file => file.name.toLowerCase().includes(filter.toLowerCase()));
  const count = data.sessions.reduce((sum, session) => sum + session.count, 0);
  const totalBytes = data.sessions.reduce((sum, session) => sum + session.bytes, 0);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark"><Icon kind="image" size={23} /></span><span>ShotSort<span className="brand-caption">A place for every screenshot.</span></span></div>
        <div className="sidebar-heading"><span>YOUR SESSIONS</span><span className="counter">{data.sessions.length}</span></div>
        <button className="new-session" disabled={!configured || busy || !desktop} onClick={() => { setName(""); setError(null); setModal("session"); }}><Icon kind="plus" size={18} /> New session</button>
        <nav className="session-nav" aria-label="Sessions">
          {data.sessions.map(session => <button key={session.id} className={`session-item ${selectedId === session.id ? "selected" : ""}`} onClick={() => select(session.id)} aria-current={selectedId === session.id ? "page" : undefined}>
            <Icon kind="folder" size={19} /><span className="session-name">{session.name}<small>{session.count} screenshot{session.count !== 1 ? "s" : ""}</small></span>
            {data.monitoring && data.activeSessionId === session.id && <span className="live-dot" title="Active session" aria-label="Active session" />}
          </button>)}
          {!data.sessions.length && <p className="sidebar-empty">Your assignment sessions<br />will appear here.</p>}
        </nav>
        <div className="sidebar-bottom"><div className="local-note"><Icon kind="check" size={16} /><span>On your laptop. In your control.</span></div><button className="settings-button" disabled={!desktop || busy || data.monitoring} onClick={folderSetup} title={data.monitoring ? "Pause the active session to change folders" : "Choose screenshot and storage folders"}><Icon kind="settings" size={18} /> Folder setup</button><span className="version">SHOTSORT / 0.1</span></div>
      </aside>
      <main className="workspace">
        <header className="topbar"><span>YOUR SCREENSHOT WORKSPACE</span><span className="offline-badge"><span /> Local storage</span></header>
        <div className="page-content">
          {!desktop && <div className="banner info" role="status">You’re viewing the interface in a browser. Folder access and screenshot routing work in the desktop app. Run <code>npm run tauri dev</code> to use them.</div>}
          {error && !modal && <div className="banner error" role="alert"><span>{error}</span><button aria-label="Dismiss error" onClick={() => setError(null)}><Icon kind="close" size={16} /></button></div>}
          {data.lastError && <div className="banner error" role="alert">{data.lastError}</div>}
          {notice && <div className="banner info" role="status">{notice}</div>}
          <div className="page-title"><div><p className="eyebrow">LESS CLUTTER. MORE FOCUS.</p><h1>{selected?.name ?? "Make room for your work."}</h1><p className="subtitle">{selected ? "Your screenshots, together in one session." : "Keep screenshots with the assignment they belong to."}</p></div>{selected && <button className="button secondary" disabled={busy || !!selected.error} onClick={() => void perform(async () => { await invoke("open_session_folder", { id: selected.id }); })}><Icon kind="folder" size={17} /> Open folder <Icon kind="arrow" size={15} /></button>}</div>
          <section className={`capture-bar ${data.monitoring ? "running" : ""}`} aria-label="Screenshot routing"><span className={`capture-icon ${data.monitoring ? "running" : ""}`}><Icon kind={data.monitoring ? "image" : "pause"} size={22} /></span><div className="capture-copy"><strong>{data.monitoring ? `Saving to ${active?.name ?? "your session"}` : "Screenshot routing is paused"}</strong><span>{data.monitoring ? (data.pendingCount ? `${data.pendingCount} screenshot${data.pendingCount === 1 ? " is" : "s are"} finishing saving…` : "Take a screenshot as usual. We’ll put it in the right folder.") : "Start a session when you’re ready. Existing files stay where they are."}</span></div>
            {data.monitoring ? <button className="button secondary" disabled={busy} onClick={() => void perform(async () => { await invoke("pause_session"); setNotice("Paused. New screenshots will stay in your screenshot source folder."); })}><Icon kind="pause" size={16} /> Pause</button> : selected && <button className="button primary" disabled={busy || !desktop || !!selected.error} onClick={() => void perform(async () => { await invoke("start_session", { id: selected.id }); })}><Icon kind="play" size={16} /> Start session</button>}
          </section>
          {loading ? <div className="empty-state"><span className="eyebrow">OPENING YOUR WORKSPACE</span><h2>Loading sessions…</h2></div> : !configured ? <section className="setup-card">
            <div className="setup-illustration" aria-hidden="true"><span className="paper paper-back"><Icon kind="image" size={30} /></span><span className="paper paper-front"><Icon kind="folder" size={36} /></span></div><span className="eyebrow">A QUICK, ONE-TIME SETUP</span><h2>Two folders. A little more order.</h2><p>Tell ShotSort where screenshots arrive and where you want your assignment sessions stored.</p>
            <div className="setup-steps"><div><span>01</span><strong>Your screenshot folder</strong><p>The folder your screenshot tool saves to.</p></div><div><span>02</span><strong>Your session storage</strong><p>A separate folder for organized assignments.</p></div></div>
            <button className="button primary" onClick={folderSetup} disabled={!desktop || busy}><Icon kind="folder" size={17} /> Choose my folders</button><small>No uploads. No changes to existing screenshots.</small>
          </section> : !selected ? <section className="empty-state"><span className="empty-icon"><Icon kind="folder" size={35} /></span><h2>Your first session starts here.</h2><p>Give your assignment a name, then start the session.<br />New screenshots will have a place to land.</p><button className="button primary" onClick={() => { setName(""); setError(null); setModal("session"); }} disabled={busy}><Icon kind="plus" size={17} /> Create a session</button></section> : <>
            <div className="session-summary"><div><span className="stat-value">{selected.count}</span><span>screenshots</span></div><span className="summary-divider" /><div><span className="stat-value">{bytes(selected.bytes)}</span><span>in this session</span></div><div className="created-date">Created {new Date(selected.createdAt).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" })}</div></div>
            {selected.error && <div className="banner error" role="alert">This session’s folder is unavailable. Its saved record is preserved. {selected.error}</div>}
            {data.monitoring && data.activeSessionId !== selected.id && <div className="switch-row"><span>Viewing this session doesn’t change where screenshots are saved.</span><button className="text-button" disabled={busy || !!selected.error} onClick={() => void perform(async () => { await invoke("start_session", { id: selected.id }); })}>Switch routing here <span aria-hidden="true">→</span></button></div>}
            <section className="files-panel" aria-label="Session screenshots"><div className="files-header"><h2>Screenshots <span>{selected.count}</span></h2><input className="search" aria-label="Find a screenshot" placeholder="Find a screenshot…" value={filter} onChange={event => setFilter(event.target.value)} /></div>
              {files.length ? <div className="table-scroll"><table><thead><tr><th>FILE NAME</th><th>SIZE</th><th>SAVED</th><th><span className="sr-only">Actions</span></th></tr></thead><tbody>{files.map(file => <tr key={file.name}><td><div className="file-name"><span className="file-icon"><Icon kind="image" size={19} /></span><button className="file-link" disabled={busy} onClick={() => void perform(async () => { await invoke("open_screenshot", { sessionId: selected.id, name: file.name }); })}>{file.name}</button></div></td><td>{bytes(file.bytes)}</td><td>{new Date(file.modifiedAt).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}</td><td><button className="icon-button" aria-label={`Open ${file.name}`} disabled={busy} onClick={() => void perform(async () => { await invoke("open_screenshot", { sessionId: selected.id, name: file.name }); })}><Icon kind="arrow" size={17} /></button></td></tr>)}</tbody></table></div> : <div className="files-empty"><span className="empty-icon"><Icon kind="image" size={30} /></span><h3>{filter ? "No matching screenshots" : "Ready for your first screenshot"}</h3><p>{filter ? "Try another filename." : data.monitoring && data.activeSessionId === selected.id ? "Save a screenshot to your source folder. It will appear here after it finishes saving." : "Start this session, then take a screenshot. We’ll handle the folder."}</p></div>}
            </section><div className="folder-path"><Icon kind="folder" size={15} /><span title={displayPath(selected.folder)}>{displayPath(selected.folder)}</span></div>
          </>}
          {configured && <footer className="workspace-footer"><span>{count} screenshot{count === 1 ? "" : "s"} · {bytes(totalBytes)} across all sessions</span><span>Minimize to keep routing. Closing ShotSort stops it.</span></footer>}
        </div>
      </main>
      <dialog ref={dialog} className="dialog" onCancel={event => { if (busy) event.preventDefault(); else { setModal(null); setError(null); } }} onClose={() => { setModal(null); setError(null); }}>
        <div className="dialog-heading"><div><p className="eyebrow">{modal === "folders" ? "YOUR FILES, YOUR FOLDERS" : "ONE ASSIGNMENT, ONE PLACE"}</p><h2>{modal === "folders" ? "Folder setup" : "Create a session"}</h2></div><button className="icon-button" aria-label="Close dialog" disabled={busy} onClick={() => { setModal(null); setError(null); }}><Icon kind="close" /></button></div>
        {error && <div className="banner error" role="alert">{error}</div>}
        {modal === "folders" ? <form onSubmit={event => { event.preventDefault(); void perform(async () => { await invoke("configure_folders", { source, destination }); setModal(null); setNotice("Folders saved. Create a session to get started; existing screenshots haven’t been moved."); }); }}>
          <label htmlFor="source-folder">Screenshot source</label><p className="field-help">Choose the dedicated folder your screenshot tool saves to. All new PNG, JPG, JPEG, and WebP files in this folder are routed.</p><div className="path-input"><input id="source-folder" placeholder="e.g. Pictures\Screenshots" value={source} required onChange={event => setSource(event.target.value)} disabled={busy} /><button type="button" className="button secondary" onClick={() => void browse("source")} disabled={busy}>Browse</button></div>
          <label htmlFor="storage-folder">Session storage</label><p className="field-help">ShotSort creates a folder here for each session. Use a separate folder, not one inside the screenshot source.</p><div className="path-input"><input id="storage-folder" placeholder="e.g. Documents\Assignments" value={destination} required onChange={event => setDestination(event.target.value)} disabled={busy} /><button type="button" className="button secondary" onClick={() => void browse("destination")} disabled={busy}>Browse</button></div>
          <div className="form-note">New screenshots are moved after they finish saving. Existing screenshots stay untouched. Moves in a synced folder, such as OneDrive, may also sync to other devices.{data.sessions.length > 0 && " Changing storage only affects future sessions; existing session folders stay where they are."}</div>
          <div className="dialog-actions"><button type="button" className="button secondary" disabled={busy} onClick={() => { setModal(null); setError(null); }}>Cancel</button><button type="submit" className="button primary" disabled={busy || !source.trim() || !destination.trim()}>{busy ? "Saving…" : "Save folders"}</button></div>
        </form> : <form onSubmit={event => { event.preventDefault(); void perform(async () => { const id = await invoke<string>("create_session", { name }); select(id); setModal(null); setNotice("Session created. Click Start session when you’re ready to save screenshots to it."); }); }}>
          <label htmlFor="session-name">Session name</label><input autoFocus id="session-name" className="name-input" placeholder="e.g. DBMS · Assignment 04" value={name} required maxLength={80} onChange={event => setName(event.target.value)} disabled={busy} /><p className="field-help">Use an assignment, subject, or project name you’ll recognize.</p><div className="form-note"><Icon kind="folder" size={18} /><span>A new session folder will be created in<br /><strong className="break-path">{displayPath(data.storageDir)}</strong></span></div><div className="dialog-actions"><button type="button" className="button secondary" disabled={busy} onClick={() => { setModal(null); setError(null); }}>Cancel</button><button type="submit" className="button primary" disabled={busy || !name.trim()}>{busy ? "Creating…" : "Create session"}</button></div>
        </form>}
      </dialog>
    </div>
  );
}

export default App;
