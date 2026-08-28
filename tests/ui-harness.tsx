// Development-only IPC fixture. Native file operations are covered by Rust tests.
import React from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";

if (!import.meta.env.DEV) throw new Error("The test harness must not run in production.");
Object.assign(window, { isTauri: true });
type FileFixture = { name: string; bytes: number; modifiedAt: number };
type SessionFixture = { id: string; name: string; folder: string; createdAt: number; files: FileFixture[] };
const state = {
  sourceDir: null as string | null,
  storageDir: null as string | null,
  managedStorage: false,
  defaultStorageDir: "C:\\ShotSort-test\\AppData\\sessions",
  activeSessionId: null as string | null,
  monitoring: false,
  sessions: [] as SessionFixture[],
};
let failStart = false;
let shotNumber = 0;
const output = document.querySelector<HTMLOutputElement>("#last-command")!;

mockIPC((command, input) => {
  const args = input as Record<string, string>;
  if (command !== "get_state") output.textContent = ` Last command: ${command}`;
  switch (command) {
    case "get_state": return {
      ...state,
      sessions: state.sessions.map(({ files, ...session }) => ({ ...session, count: files.length, bytes: files.reduce((sum, file) => sum + file.bytes, 0), error: null })),
      screenshots: state.sessions.find(session => session.id === args.selectedId)?.files ?? [],
      pendingCount: 0,
      lastError: null,
    };
    case "choose_folder": return "C:\\ShotSort-test\\Screenshots";
    case "configure_folders":
      if (args.source === args.destination) throw new Error("Choose separate screenshot and storage folders.");
      state.sourceDir = args.source;
      state.managedStorage = !args.destination;
      state.storageDir = args.destination || state.defaultStorageDir;
      return;
    case "create_quick_session": {
      const number = state.sessions.length + 1;
      const id = `session-${number}`;
      state.sessions.unshift({ id, name: `Quick session ${number}`, folder: `${state.storageDir}\\${id}`, createdAt: Date.now(), files: [] });
      return id;
    }
    case "create_session": {
      const id = `session-${state.sessions.length + 1}`;
      state.sessions.unshift({ id, name: args.name.trim(), folder: `${state.storageDir}\\${id}`, createdAt: Date.now(), files: [] });
      return id;
    }
    case "start_session":
      if (failStart) { failStart = false; throw new Error("This session folder is missing or has moved."); }
      state.activeSessionId = args.id; state.monitoring = true; return;
    case "pause_session": state.monitoring = false; return;
    case "open_screenshot": case "open_session_folder": return;
    default: throw new Error(`Unexpected test command: ${command}`);
  }
});
document.querySelector("#capture-fixture")!.addEventListener("click", () => {
  const session = state.sessions.find(session => session.id === state.activeSessionId);
  if (state.monitoring && session) session.files.unshift({ name: `Screenshot-${++shotNumber}.png`, bytes: 128 * 1024, modifiedAt: Date.now() });
});
document.querySelector("#fail-start")!.addEventListener("click", () => { failStart = true; });
const { default: App } = await import("../src/App");
createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);
