# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

# ShotSort
# 📸 ShotSort

**ShotSort** is a lightweight cross-platform desktop application that automatically organizes screenshots into user-defined assignment folders.

Instead of manually sorting or deleting hundreds of screenshots at the end of the semester, simply select your current assignment in ShotSort and continue taking screenshots normally. The application monitors your default screenshots directory and instantly moves new screenshots to the active assignment folder.

---

## ✨ Features

- 📁 Create assignment folders directly from the app
- 🔄 Switch active assignment with a single click
- 👀 Real-time monitoring of the system screenshots directory
- 🚀 Automatic screenshot organization
- 🗑️ Rename and delete assignment folders
- 📂 Open assignment folders directly from the application
- 💾 Persistent configuration and assignment history
- 🌙 Lightweight, modern desktop interface

---

## 🛠️ Tech Stack

### Frontend
- React
- TypeScript
- Tailwind CSS
- shadcn/ui

### Desktop Framework
- Tauri v2

### Backend
- Rust

### Libraries
- `notify` – File system monitoring
- `serde` / `serde_json` – Configuration management
- `tauri-plugin-fs` – File system operations
- `tauri-plugin-dialog` – Native dialogs
- `tauri-plugin-shell` – Opening folders and system integration

---

## ⚙️ How It Works

1. Launch ShotSort.
2. Create an assignment folder.
3. Select it as the active assignment.
4. Continue taking screenshots normally.
5. ShotSort automatically detects new screenshots and moves them into the selected assignment folder.

---

## 📂 Project Structure

```
ShotSort/
├── src/                # React frontend
├── src-tauri/          # Rust backend
├── public/
├── README.md
└── package.json
```

---

## 🚧 Roadmap

- [x] Assignment management
- [x] Automatic screenshot organization
- [ ] System tray support
- [ ] Desktop notifications
- [ ] Export screenshots to PDF
- [ ] Keyboard shortcuts for quick assignment switching
- [ ] Screenshot statistics

---

## 📄 License

This project is licensed under the MIT License.