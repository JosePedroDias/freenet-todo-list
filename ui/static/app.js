import init, {
  connect as wzConnect,
  add_task as wzAddTask,
  toggle_task as wzToggleTask,
  save_note as wzSaveNote,
  load_note as wzLoadNote,
  list_notes as wzListNotes,
  rotate_key as wzRotateKey,
} from "./todo_ui.js";

const todo = {
  connect: wzConnect,
  add_task: wzAddTask,
  toggle_task: wzToggleTask,
  save_note: wzSaveNote,
  load_note: wzLoadNote,
  list_notes: wzListNotes,
  rotate_key: wzRotateKey,
};

let state = { tasks: {}, toggles: {} };
let knownNotes = {};
let pendingNotes = {};

window.todoOnState = (json) => {
  const incoming = JSON.parse(json);
  if (Object.keys(incoming.tasks).length >= Object.keys(state.tasks).length) {
    state = incoming;
    todo.list_notes();
  } else {
    for (const [k, v] of Object.entries(incoming.tasks || {})) state.tasks[k] = v;
    for (const [k, v] of Object.entries(incoming.toggles || {})) {
      const ex = state.toggles[k];
      if (!ex || v.ts > ex.ts || (v.ts === ex.ts && lexicmp(v.signature, ex.signature) > 0)) {
        state.toggles[k] = v;
      }
    }
  }
  render();
};

window.todoOnStatus = (status) => {
  const pill = document.getElementById("status-pill");
  pill.textContent = status;
  pill.className = "status " + (status === "connected" ? "connected" : "");
};

window.todoOnDelegate = (json) => {
  const resp = JSON.parse(json);
  switch (resp.type) {
    case "Saved":
      knownNotes[resp.task_id] = pendingNotes[resp.task_id] || "";
      delete pendingNotes[resp.task_id];
      render();
      break;
    case "Note":
      if (resp.text !== null && resp.text !== undefined) knownNotes[resp.task_id] = resp.text;
      if (pendingNotes[resp.task_id] === undefined || pendingNotes[resp.task_id] === "") {
        pendingNotes[resp.task_id] = knownNotes[resp.task_id] || "";
      }
      render();
      break;
    case "NotesList":
      for (const id of resp.task_ids) todo.load_note(BigInt(id));
      break;
  }
};

window.todoOnIdentity = (shortId) => {
  document.getElementById("id-label").textContent = shortId + "…";
  document.getElementById("new-task").disabled = false;
  document.getElementById("new-task").placeholder = "add a task…";
  document.getElementById("add-btn").disabled = false;
};

function addTask() {
  const input = document.getElementById("new-task");
  if (input.value.trim()) { todo.add_task(input.value.trim()); input.value = ""; }
}
function toggleTask(id) { todo.toggle_task(BigInt(id)); }
function saveNote(id) {
  const ta = document.getElementById("note-" + id);
  if (ta) { pendingNotes[id] = ta.value; todo.save_note(BigInt(id), ta.value); }
}
function expandNote(id) {
  const area = document.getElementById("note-area-" + id);
  if (area) { delete pendingNotes[id]; render(); }
  else {
    if (knownNotes[id] === undefined && pendingNotes[id] === undefined) todo.load_note(BigInt(id));
    pendingNotes[id] = pendingNotes[id] || knownNotes[id] || "";
    render();
    const ta = document.getElementById("note-" + id);
    if (ta) ta.focus();
  }
}

function render() {
  const ids = Object.keys(state.tasks).map(Number).sort((a, b) => a - b);
  const list = document.getElementById("task-list");
  if (ids.length === 0) { list.innerHTML = '<div class="empty">no tasks yet — add one above</div>'; return; }
  list.innerHTML = ids.map(id => {
    const task = state.tasks[id];
    const toggle = state.toggles[id];
    const done = toggle ? toggle.done : false;
    const authorShort = (task.author || []).slice(0, 4).map(b => b.toString(16).padStart(2, "0")).join("");
    const expanded = pendingNotes[id] !== undefined;
    const noteText = pendingNotes[id] ?? knownNotes[id] ?? "";
    return `<li>
      <div class="task-head">
        <input type="checkbox" id="cb-${id}" ${done ? "checked" : ""} onchange="toggleTask(${id})" />
        <span class="task-text ${done ? "done" : ""}">${esc(task.text)}</span>
        <button class="btn-note ${expanded ? "active" : ""}" onclick="expandNote(${id})">${expanded ? "− close" : "note"}</button>
      </div>
      <div class="task-meta">#${id} · by ${authorShort}… · ${new Date(task.created_at).toLocaleString()}</div>
      ${expanded ? `<div class="note-area" id="note-area-${id}">
        <textarea id="note-${id}" placeholder="private note (stored via delegate)">${esc(noteText)}</textarea>
        <button class="btn-save" onclick="saveNote(${id})">save note</button>
      </div>` : ""}
    </li>`;
  }).join("");
}

function esc(s) { return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;"); }
function lexicmp(a, b) { const n = Math.min(a.length, b.length); for (let i = 0; i < n; i++) if (a[i] !== b[i]) return a[i] - b[i]; return a.length - b.length; }

document.getElementById("add-btn").addEventListener("click", addTask);
document.getElementById("new-task").addEventListener("keydown", (e) => { if (e.key === "Enter") addTask(); });
document.getElementById("rotate-key").addEventListener("click", () => { if (confirm("Rotate identity key?")) todo.rotate_key(); });

window.toggleTask = toggleTask;
window.saveNote = saveNote;
window.expandNote = expandNote;

init().then(() => { console.log("[todo] wasm loaded"); todo.connect(); })
  .catch(e => { console.error("[todo] wasm init failed:", e); document.body.innerHTML = "<h1>Failed to load</h1><pre>" + e + "</pre>"; });
