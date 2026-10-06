const { invoke } = window.__TAURI__.core;
const { open, confirm } = window.__TAURI__.dialog;

// Inline icons (Lucide-style strokes) so the UI ships no icon font or network
// fetch. Static markup uses <i data-icon="name"> placeholders, swapped below.
const ICON_PATHS = {
  search: '<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>',
  pencil: '<path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/>',
  chevron: '<path d="m6 9 6 6 6-6"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/>',
  moon: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>',
  refresh: '<path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/>',
  reply: '<path d="m9 17-5-5 5-5"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>',
  forward: '<path d="m15 17 5-5-5-5"/><path d="M4 18v-2a4 4 0 0 1 4-4h12"/>',
  archive: '<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>',
  trash: '<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>',
  undo: '<path d="M3 7v6h6"/><path d="M21 17a9 9 0 0 0-9-9 9 9 0 0 0-6 2.3L3 13"/>',
  send: '<path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/>',
  plus: '<path d="M5 12h14M12 5v14"/>',
  sliders: '<path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6"/>',
  check: '<path d="M20 6 9 17l-5-5"/>',
  paperclip: '<path d="m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48"/>',
  download: '<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="m7 10 5 5 5-5"/><path d="M12 15V3"/>',
  file: '<path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z"/><path d="M14 2v6h6"/>',
  mail: '<rect width="20" height="16" x="2" y="4" rx="2"/><path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"/>',
};

function icon(name) {
  return `<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICON_PATHS[name]}</svg>`;
}

document.querySelectorAll("i[data-icon]").forEach((el) => (el.outerHTML = icon(el.dataset.icon)));

const IS_MAC = /Mac/i.test(navigator.userAgent);
const MOD_LABEL = IS_MAC ? "⌘" : "Ctrl+";
document.querySelectorAll("[data-mod-key]").forEach((el) => (el.textContent = `${MOD_LABEL}${el.dataset.modKey}`));

// Theme: defaults to the OS preference, overridden once the user picks explicitly.
const THEME_KEY = "tinymail-theme";

function systemPrefersDark() {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function isDarkActive() {
  const stored = localStorage.getItem(THEME_KEY);
  return stored ? stored === "dark" : systemPrefersDark();
}

function applyTheme() {
  const stored = localStorage.getItem(THEME_KEY);
  if (stored) document.documentElement.setAttribute("data-theme", stored);
  else document.documentElement.removeAttribute("data-theme");
  const active = isDarkActive() ? "dark" : "light";
  document.querySelectorAll("[data-theme-choice]").forEach((btn) => btn.classList.toggle("active", btn.dataset.themeChoice === active));
}

function setTheme(choice) {
  localStorage.setItem(THEME_KEY, choice);
  applyTheme();
  // The open message's HTML body lives in a sandboxed iframe with its own
  // document, so it doesn't pick up the new theme via CSS — re-render it.
  if (currentMessage?.body_html) {
    renderHtmlBody(document.getElementById("message-body-frame"), currentMessage.body_html);
  }
}

document.querySelectorAll("[data-theme-choice]").forEach((btn) => btn.addEventListener("click", () => setTheme(btn.dataset.themeChoice)));
window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", applyTheme);
applyTheme();

// Status toast: transient bottom-left feedback for actions that run in the
// background (send, delete, archive, ...) instead of blocking the UI on them.
const toastEl = document.getElementById("toast");
let toastHideTimer = null;

function showToast(text, { autoHideMs } = {}) {
  clearTimeout(toastHideTimer);
  toastEl.textContent = text;
  toastEl.classList.remove("hidden");
  if (autoHideMs) toastHideTimer = setTimeout(() => toastEl.classList.add("hidden"), autoHideMs);
}

// Every IMAP-backed command takes the username of the account it acts on —
// all state below is keyed by it, so async results that land after the user
// switched accounts (or folders) update their cache without touching the view.
let accounts = []; // [{ username, imap_host, ..., expired }] in switcher order
let currentAccount = null; // username of the account being viewed
let currentFolder = "INBOX";
let currentMessage = null; // { account, folder, uid, from, to, cc, date, subject, body, body_html }
let attachmentPaths = [];
let searchQuery = "";
let openToken = 0; // bumped per message open, so a slow fetch can't overwrite a newer selection
const folderCache = {}; // account -> folder -> messages[], avoids refetching on every nav click
const messageDetailCache = {}; // "account:folder:uid" -> last-fetched MessageDetail, so reopening a message already viewed this session is instant
const knownUnreadUids = {}; // account -> Set of unread INBOX uids; baseline for new-mail notifications

const FOLDERS = ["INBOX", "SENT", "DRAFTS", "ARCHIVE", "TRASH"];
const ACTIVE_ACCOUNT_KEY = "tinymail-active-account";
const POLL_INTERVAL_MS = 60000;
const AVATAR_COLORS = ["#5b7aa8", "#5f8a6e", "#9a7a56", "#80699e", "#4f8486", "#a5666b", "#6f7b88"];

const setupView = document.getElementById("setup-view");
const appView = document.getElementById("app-view");
const setupTitle = document.getElementById("setup-title");
const setupError = document.getElementById("setup-error");
const accountForm = document.getElementById("account-form");
const cancelSetupBtn = document.getElementById("cancel-setup-btn");
const messageList = document.getElementById("message-list");
const messageDetail = document.getElementById("message-detail");
const refreshBtn = document.getElementById("refresh-btn");
const inboxUnreadBadge = document.getElementById("inbox-unread-badge");
const accountBtn = document.getElementById("account-btn");
const accountMenu = document.getElementById("account-menu");
const accountMenuList = document.getElementById("account-menu-list");
const accountsModal = document.getElementById("accounts-modal");
const accountsManageList = document.getElementById("accounts-manage-list");
const searchInput = document.getElementById("search-input");

function folders(account) {
  return (folderCache[account] ??= {});
}

function isViewing(account, folder) {
  return account === currentAccount && folder === currentFolder;
}

function usableAccounts() {
  return accounts.filter((a) => !a.expired);
}

// Persisted, per-account message-list cache (localStorage survives app
// restarts, unlike `folderCache`), so a returning user sees their last-known
// mail instantly instead of a "Loading..." screen while IMAP reconnects.
// Keyed by account username so switching accounts can never show a stale
// list from another one.
function cacheKey(account, folder) {
  return `tinymail-cache:${account}:${folder}`;
}

function persistFolderCache(account, folder) {
  const messages = folderCache[account]?.[folder];
  if (!messages) return;
  try {
    localStorage.setItem(cacheKey(account, folder), JSON.stringify(messages));
  } catch (err) {
    console.error(err); // best-effort cache — storage full/unavailable is not fatal
  }
}

function loadPersistedFolderCache(account, folder) {
  try {
    const raw = localStorage.getItem(cacheKey(account, folder));
    return raw ? JSON.parse(raw) : null;
  } catch (err) {
    return null;
  }
}

function removePersistedFolderCache(account, folder) {
  localStorage.removeItem(cacheKey(account, folder));
}

function inboxUnreadCount(account) {
  const inbox = folderCache[account]?.INBOX ?? loadPersistedFolderCache(account, "INBOX") ?? [];
  return inbox.filter((m) => m.unread).length;
}

function avatarColor(username) {
  let hash = 0;
  for (const ch of username) hash = (hash * 31 + ch.codePointAt(0)) >>> 0;
  return AVATAR_COLORS[hash % AVATAR_COLORS.length];
}

function avatarInitial(username) {
  return (username.match(/[a-z0-9]/i)?.[0] ?? "?").toUpperCase();
}

function avatarHtml(username, extraClass = "") {
  return `<span class="avatar ${extraClass}" style="--av:${avatarColor(username)}">${avatarInitial(username)}</span>`;
}

// ---- Account switcher -------------------------------------------------------

function renderAccountSwitcher() {
  if (!currentAccount) return;
  const avatar = document.getElementById("account-avatar");
  avatar.style.setProperty("--av", avatarColor(currentAccount));
  avatar.textContent = avatarInitial(currentAccount);
  document.getElementById("account-name").textContent = currentAccount;

  const count = inboxUnreadCount(currentAccount);
  inboxUnreadBadge.textContent = String(count);
  inboxUnreadBadge.classList.toggle("hidden", count === 0);

  accountMenuList.innerHTML = accounts
    .map((a, i) => {
      const active = a.username === currentAccount;
      const unread = a.expired ? 0 : inboxUnreadCount(a.username);
      const status = a.expired
        ? `<span class="tag">Reconnect</span>`
        : unread
          ? `<span class="count${active ? " on" : ""}">${unread}</span>`
          : "";
      return `<button type="button" class="menu-item" role="menuitem" data-account="${escapeHtml(a.username)}">
        ${avatarHtml(a.username)}
        <span class="email">${escapeHtml(a.username)}</span>
        ${status}
        <span class="check">${active ? icon("check") : ""}</span>
        <span class="shortcut">${i < 9 ? `${MOD_LABEL}${i + 1}` : ""}</span>
      </button>`;
    })
    .join("");
  accountMenuList.querySelectorAll("[data-account]").forEach((btn) => btn.addEventListener("click", () => switchAccount(btn.dataset.account)));
}

function toggleAccountMenu(force) {
  const show = force ?? accountMenu.classList.contains("hidden");
  accountMenu.classList.toggle("hidden", !show);
  accountBtn.setAttribute("aria-expanded", String(show));
}

accountBtn.addEventListener("click", (e) => {
  e.stopPropagation();
  toggleAccountMenu();
});
document.addEventListener("click", (e) => {
  if (!accountMenu.contains(e.target)) toggleAccountMenu(false);
});
document.getElementById("add-account-btn").addEventListener("click", () => showSetup());
document.getElementById("manage-accounts-btn").addEventListener("click", openManageAccounts);

function switchAccount(username) {
  toggleAccountMenu(false);
  const account = accounts.find((a) => a.username === username);
  if (!account) return;
  if (account.expired) {
    showSetup({ account, message: "Your session expired — please re-enter your password." });
    return;
  }
  currentAccount = username;
  localStorage.setItem(ACTIVE_ACCOUNT_KEY, username);
  currentMessage = null;
  searchQuery = "";
  searchInput.value = "";
  renderAccountSwitcher();
  openFolder(currentFolder);
}

// ---- Setup / add / reconnect ------------------------------------------------

function showSetup({ account = null, message = "" } = {}) {
  toggleAccountMenu(false);
  accountsModal.classList.add("hidden");
  accountForm.reset();
  if (account) prefillSetupForm(account);
  setupTitle.textContent = account ? "Reconnect your account" : accounts.length ? "Add an account" : "Connect your account";
  setupError.textContent = message;
  cancelSetupBtn.classList.toggle("hidden", !usableAccounts().some((a) => a.username !== account?.username));
  appView.classList.add("hidden");
  setupView.classList.remove("hidden");
  accountForm.elements[account ? "password" : "username"].focus();
}

function prefillSetupForm(account) {
  for (const field of ["username", "imap_host", "imap_port", "smtp_host", "smtp_port", "sent_folder", "drafts_folder", "archive_folder", "trash_folder"]) {
    accountForm.elements[field].value = account[field];
  }
}

cancelSetupBtn.addEventListener("click", () => {
  const usable = usableAccounts();
  const fallback = usable.find((a) => a.username === currentAccount) ?? usable[0];
  if (fallback) enterApp(fallback.username);
});

accountForm.addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = new FormData(e.target);
  const account = {
    username: form.get("username").trim(),
    imap_host: form.get("imap_host").trim(),
    imap_port: Number(form.get("imap_port")),
    smtp_host: form.get("smtp_host").trim(),
    smtp_port: Number(form.get("smtp_port")),
    sent_folder: form.get("sent_folder") || "Sent",
    drafts_folder: form.get("drafts_folder") || "Drafts",
    archive_folder: form.get("archive_folder") || "Archive",
    trash_folder: form.get("trash_folder") || "Trash",
  };
  const password = form.get("password");
  setupError.textContent = "";
  try {
    await invoke("save_account", { account, password });
    // Saved settings may differ from before (reconnect/edit) — drop anything cached under the old ones.
    forgetAccountCaches(account.username);
    accounts = await invoke("list_accounts");
    enterApp(account.username);
  } catch (err) {
    setupError.textContent = String(err);
  }
});

// ---- Manage accounts --------------------------------------------------------

function openManageAccounts() {
  toggleAccountMenu(false);
  accountsManageList.innerHTML = accounts
    .map(
      (a) => `<li class="manage-row">
        ${avatarHtml(a.username, "lg")}
        <div class="who">
          <div class="email">${escapeHtml(a.username)}</div>
          <div class="host">${escapeHtml(a.imap_host)}:${a.imap_port} · ${escapeHtml(a.smtp_host)}:${a.smtp_port}</div>
        </div>
        ${a.expired ? '<span class="tag">Expired</span>' : ""}
        <button type="button" class="btn" data-edit="${escapeHtml(a.username)}">${a.expired ? "Reconnect" : "Edit"}</button>
        <button type="button" class="btn danger" data-remove="${escapeHtml(a.username)}">Remove</button>
      </li>`
    )
    .join("");
  accountsManageList.querySelectorAll("[data-edit]").forEach((btn) =>
    btn.addEventListener("click", () => showSetup({ account: accounts.find((a) => a.username === btn.dataset.edit) }))
  );
  accountsManageList.querySelectorAll("[data-remove]").forEach((btn) => btn.addEventListener("click", () => removeAccount(btn.dataset.remove)));
  accountsModal.classList.remove("hidden");
}

document.getElementById("close-accounts-btn").addEventListener("click", () => accountsModal.classList.add("hidden"));
document.getElementById("manage-add-btn").addEventListener("click", () => showSetup());

function forgetAccountCaches(username) {
  delete folderCache[username];
  delete knownUnreadUids[username];
  for (const folder of FOLDERS) removePersistedFolderCache(username, folder);
  for (const key of Object.keys(messageDetailCache)) {
    if (key.startsWith(`${username}:`)) delete messageDetailCache[key];
  }
}

async function removeAccount(username) {
  const ok = await confirm(`Remove ${username} from tinymail? Mail on the server is not affected.`, { title: "tinymail", kind: "warning" });
  if (!ok) return;
  try {
    await invoke("remove_account", { account: username });
  } catch (err) {
    showToast(`Failed: ${err}`, { autoHideMs: 4000 });
    return;
  }
  forgetAccountCaches(username);
  accounts = await invoke("list_accounts");
  if (accounts.length === 0) {
    currentAccount = null;
    localStorage.removeItem(ACTIVE_ACCOUNT_KEY);
    showSetup();
    return;
  }
  if (username === currentAccount) {
    const next = usableAccounts()[0];
    if (next) switchAccount(next.username);
    else {
      showSetup({ account: accounts[0], message: "Your session expired — please re-enter your password." });
      return;
    }
  } else {
    renderAccountSwitcher();
  }
  openManageAccounts();
  showToast(`Removed ${username}`, { autoHideMs: 2000 });
}

// ---- App lifecycle ----------------------------------------------------------

let appStarted = false;

function enterApp(username) {
  setupView.classList.add("hidden");
  appView.classList.remove("hidden");
  switchAccount(username);
  if (appStarted) return;
  appStarted = true;
  window.__TAURI__.app.getVersion().then((version) => {
    document.getElementById("app-version").textContent = `tinymail v${version}`;
  });
  // Fetch the other accounts' inboxes up front so the switcher shows real
  // unread counts (and new-mail baselines exist) before the first poll.
  for (const a of usableAccounts()) {
    if (a.username !== username) checkForNewMail(a.username);
  }
  setInterval(() => usableAccounts().forEach((a) => checkForNewMail(a.username)), POLL_INTERVAL_MS);
}

// The window starts hidden (see tauri.conf.json) so the OS never paints a
// blank/grey native window before the webview has anything to show — it's
// revealed only once we know what to display (setup form or app view),
// already fully rendered.
async function showWindow() {
  try {
    const win = window.__TAURI__.window.getCurrentWindow();
    await win.show();
    // show() alone can leave the window visible but behind other apps
    // (notably on macOS, where it doesn't imply activation) — focus it so it
    // actually comes to the front instead of needing a manual click to raise.
    await win.setFocus();
  } catch (err) {
    console.error(err);
  }
}

async function init() {
  try {
    accounts = await invoke("list_accounts");
    if (accounts.length === 0) {
      showSetup();
      return;
    }
    const saved = localStorage.getItem(ACTIVE_ACCOUNT_KEY);
    const initial = accounts.find((a) => a.username === saved && !a.expired) ?? usableAccounts()[0] ?? accounts[0];
    if (initial.expired) {
      showSetup({ account: initial, message: "Your session expired — please re-enter your password." });
      return;
    }
    enterApp(initial.username);
  } catch (e) {
    console.error(e);
  } finally {
    showWindow();
  }
}

// ---- Folders & message list -------------------------------------------------

document.querySelectorAll(".folder-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".folder-btn").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    currentFolder = btn.dataset.folder;
    openFolder(currentFolder);
  });
});

refreshBtn.addEventListener("click", () => loadFolder(currentAccount, currentFolder, { force: true }));

searchInput.addEventListener("input", () => {
  searchQuery = searchInput.value;
  renderMessageList();
});

// Opens a folder for viewing: if it's already loaded this session, defer to
// the normal cache-or-fetch behavior in loadFolder. Otherwise, paint a stale
// copy from the on-disk cache (if any) immediately, then always refresh from
// the server — silently, so a good cache doesn't get blown away by a
// "Loading..." flash.
function openFolder(folder) {
  const account = currentAccount;
  if (folders(account)[folder]) {
    return loadFolder(account, folder);
  }
  const persisted = loadPersistedFolderCache(account, folder);
  if (persisted) {
    folders(account)[folder] = persisted;
    renderMessageList();
  }
  return loadFolder(account, folder, { force: true, silent: !!persisted });
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// Date strings arrive as IMAP envelope / RFC3339 header text, in whatever
// timezone the sender's server stamped them; both helpers render them in the
// viewer's local time. The list uses a compact form ("14:05", "21 Sep",
// "21 Sep 2024"); the reading pane shows the full "21 Sep 2026, 14:05".
const pad2 = (n) => String(n).padStart(2, "0");

function formatShortDate(raw) {
  if (!raw) return "";
  const d = new Date(raw);
  if (isNaN(d.getTime())) return raw;
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  const day = `${d.getDate()} ${MONTHS[d.getMonth()]}`;
  return d.getFullYear() === now.getFullYear() ? day : `${day} ${d.getFullYear()}`;
}

function formatLongDate(raw) {
  if (!raw) return "";
  const d = new Date(raw);
  if (isNaN(d.getTime())) return raw;
  return `${d.getDate()} ${MONTHS[d.getMonth()]} ${d.getFullYear()}, ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

function formatLocalDate(raw) {
  if (!raw) return "";
  const parsed = new Date(raw);
  if (isNaN(parsed.getTime())) return raw;
  return parsed.toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

// Renders the current account + folder's cached list (filtered by search).
function renderMessageList() {
  renderAccountSwitcher();
  const messages = folderCache[currentAccount]?.[currentFolder];
  if (!messages) return;

  const q = searchQuery.trim().toLowerCase();
  const shown = q ? messages.filter((m) => m.from.toLowerCase().includes(q) || m.subject.toLowerCase().includes(q)) : messages;
  if (shown.length === 0) {
    messageList.innerHTML = `<p class='placeholder'>${q ? "No matching messages" : "No messages"}</p>`;
    return;
  }
  const activeUid = currentMessage?.account === currentAccount && currentMessage?.folder === currentFolder ? currentMessage.uid : null;
  messageList.innerHTML = "";
  for (const m of shown) {
    const item = document.createElement("div");
    item.className = `message-item${m.unread ? " unread" : ""}${m.uid === activeUid ? " active" : ""}`;
    item.dataset.uid = String(m.uid);
    item.innerHTML = `<span class="unread-dot"></span>
      <div>
        <div class="message-row"><span class="from">${escapeHtml(m.from)}</span><span class="date">${escapeHtml(formatShortDate(m.date))}</span></div>
        <div class="subject">${escapeHtml(m.subject || "(no subject)")}</div>
      </div>`;
    item.addEventListener("click", () => selectMessageItem(item));
    messageList.appendChild(item);
  }
}

function selectMessageItem(item) {
  messageList.querySelectorAll(".message-item").forEach((el) => el.classList.remove("active"));
  item.classList.add("active");
  item.scrollIntoView({ block: "nearest" });
  showMessage(currentAccount, currentFolder, Number(item.dataset.uid));
}

function moveSelection(delta) {
  const items = [...messageList.querySelectorAll(".message-item")];
  if (items.length === 0) return;
  const current = items.findIndex((el) => el.classList.contains("active"));
  const next = current === -1 ? 0 : Math.min(items.length - 1, Math.max(0, current + delta));
  if (next !== current) selectMessageItem(items[next]);
}

function clearDetail() {
  currentMessage = null;
  messageDetail.innerHTML = `<div class="detail-empty">${icon("mail")}<span>Select a message</span><span class="hint"><kbd>J</kbd><kbd>K</kbd> to move · <kbd>C</kbd> to compose</span></div>`;
}

async function loadFolder(account, folder, { force = false, silent = false } = {}) {
  if (isViewing(account, folder)) clearDetail();

  if (!force && folders(account)[folder]) {
    if (isViewing(account, folder)) renderMessageList();
    return;
  }

  if (!silent && isViewing(account, folder)) messageList.innerHTML = "<p class='placeholder'>Loading...</p>";
  refreshBtn.classList.add("spinning");
  try {
    const messages = await invoke("list_messages", { account, folder });
    if (!folderCache[account]) return; // account removed while loading
    folderCache[account][folder] = messages;
    persistFolderCache(account, folder);
    prefetch(account, folder, messages.slice(0, PREFETCH_TOP).map((m) => m.uid));
    if (folder === "INBOX" && !knownUnreadUids[account]) {
      knownUnreadUids[account] = new Set(messages.filter((m) => m.unread).map((m) => m.uid));
    }
    if (isViewing(account, folder)) renderMessageList();
    else renderAccountSwitcher();
  } catch (err) {
    if (silent || !isViewing(account, folder)) {
      console.error(err); // keep showing the cached list already on screen instead of replacing it with an error
    } else {
      messageList.innerHTML = `<p class="error placeholder">${escapeHtml(String(err))}</p>`;
    }
  } finally {
    refreshBtn.classList.remove("spinning");
  }
}

function invalidateFolder(account, folder) {
  if (folderCache[account]) delete folderCache[account][folder];
  removePersistedFolderCache(account, folder);
  if (isViewing(account, folder)) loadFolder(account, folder, { force: true });
}

async function confirmAndMove(question, command, destFolder, statusLabels) {
  if (await confirm(question, { title: "tinymail", kind: "warning" })) {
    moveCurrentMessage(command, destFolder, statusLabels);
  }
}

// Optimistic: drops the message from the UI and shows an in-progress toast
// immediately, then runs the IMAP command in the background instead of
// blocking on it. On failure, the folder is force-refreshed to reconcile the
// view with what the server actually did.
function moveCurrentMessage(command, destFolder, { inProgress = "Working...", done = "Done" } = {}) {
  if (!currentMessage) return;
  const { account, folder, uid } = currentMessage;
  clearDetail();

  const cache = folderCache[account];
  if (cache?.[folder]) {
    cache[folder] = cache[folder].filter((m) => m.uid !== uid);
    persistFolderCache(account, folder);
    if (isViewing(account, folder)) renderMessageList();
    else renderAccountSwitcher();
  }
  if (destFolder) {
    if (cache) delete cache[destFolder];
    removePersistedFolderCache(account, destFolder);
  }

  showToast(inProgress);
  invoke(command, { account, folder, uid })
    .then(() => showToast(done, { autoHideMs: 2000 }))
    .catch((err) => {
      showToast(`Failed: ${err}`, { autoHideMs: 4000 });
      if (isViewing(account, folder)) loadFolder(account, folder, { force: true });
    });
}

// ---- Message view -----------------------------------------------------------

async function showMessage(account, folder, uid) {
  const token = ++openToken;
  const key = `${account}:${folder}:${uid}`;
  // A UID's content never changes on an IMAP server, so a message fetched
  // once this session is rendered straight from memory with no round trip.
  const cached = messageDetailCache[key];
  if (cached) {
    renderMessage(account, folder, uid, cached);
    markMessageRead(account, folder, uid);
    prefetchAround(uid);
    return;
  }

  // Paint the header from the list entry right away, so the pane responds
  // to the click while the body loads (usually from the backend's prefetch
  // cache, so this is brief).
  currentMessage = null;
  const summary = folderCache[account]?.[folder]?.find((m) => m.uid === uid);
  messageDetail.innerHTML = summary
    ? `<div class="detail-scroll">
        <h1 class="detail-subject">${escapeHtml(summary.subject || "(no subject)")}</h1>
        <div class="detail-meta"><div><div class="from-addr">${escapeHtml(summary.from)}</div></div><div class="meta-date">${escapeHtml(formatLongDate(summary.date))}</div></div>
        <p class="loading-body">Loading…</p>
      </div>`
    : "<p class='placeholder'>Loading…</p>";
  // Holding J/K fires an open per row; a short pause lets the rows the user
  // is skipping past drop out before they queue a fetch on the connection.
  await new Promise((r) => setTimeout(r, 60));
  if (token !== openToken) return;
  try {
    const msg = await invoke("get_message", { account, folder, uid });
    messageDetailCache[key] = msg;
    if (token !== openToken) return; // user opened something else meanwhile
    renderMessage(account, folder, uid, msg);
    // mark_read runs after, once the content is already on screen — IMAP
    // allows one command in flight per connection, so running it alongside
    // get_message would only delay the fetch the user is waiting on.
    markMessageRead(account, folder, uid);
    prefetchAround(uid);
  } catch (err) {
    if (token === openToken) messageDetail.innerHTML = `<p class="error placeholder">${escapeHtml(String(err))}</p>`;
  }
}

// Asks the backend to download the next few messages into its cache (on a
// separate background IMAP connection), so opening them is instant.
const PREFETCH_TOP = 12;
const PREFETCH_AHEAD = 4;

function prefetch(account, folder, uids) {
  const missing = uids.filter((uid) => !messageDetailCache[`${account}:${folder}:${uid}`]);
  if (missing.length) invoke("prefetch_messages", { account, folder, uids: missing }).catch((err) => console.error(err));
}

function prefetchAround(uid) {
  const messages = folderCache[currentAccount]?.[currentFolder];
  if (!messages) return;
  const i = messages.findIndex((m) => m.uid === uid);
  if (i === -1) return;
  const near = [...messages.slice(i + 1, i + 1 + PREFETCH_AHEAD), ...messages.slice(Math.max(0, i - 1), i)];
  prefetch(currentAccount, currentFolder, near.map((m) => m.uid));
}

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function actionButton(id, iconName, label, key, extraClass = "") {
  return `<button type="button" id="${id}" class="btn ${extraClass}" data-key="${key}">${icon(iconName)}${label}<kbd>${key.toUpperCase()}</kbd></button>`;
}

function renderMessage(account, folder, uid, msg) {
  currentMessage = { account, folder, uid, from: msg.from, to: msg.to, cc: msg.cc, date: msg.date, subject: msg.subject, body: msg.body, body_html: msg.body_html };
  const bodyHtml = msg.body_html
    ? `<iframe id="message-body-frame" sandbox="allow-same-origin" title="Message body"></iframe>`
    : `<div class="body">${formatBody(msg.body)}</div>`;
  const attachmentsHtml = msg.attachments
    .map(
      (a) =>
        `<li class="attachment-item" data-filename="${escapeHtml(a.filename)}" data-content-type="${escapeHtml(a.content_type)}" title="${escapeHtml(a.filename)}">
          <span class="attachment-preview">${icon("file")}</span>
          <div class="attachment-meta"><div class="attachment-name">${escapeHtml(a.filename)}</div><div class="attachment-size">${formatSize(a.size)}</div></div>
          <button type="button" class="icon-btn" data-filename="${escapeHtml(a.filename)}" title="Save">${icon("download")}</button>
        </li>`
    )
    .join("");
  const moveAction =
    folder === "ARCHIVE"
      ? actionButton("unarchive-btn", "undo", "Unarchive", "e")
      : folder === "TRASH"
        ? actionButton("restore-btn", "undo", "Restore", "e")
        : actionButton("archive-btn", "archive", "Archive", "e");
  const deleteAction =
    folder === "TRASH" ? actionButton("delete-forever-btn", "trash", "Delete forever", "#", "danger") : actionButton("delete-btn", "trash", "Trash", "#", "danger");

  messageDetail.innerHTML = `
    <div class="detail-scroll">
      <h1 class="detail-subject">${escapeHtml(msg.subject || "(no subject)")}</h1>
      <div class="detail-meta">
        <div>
          <div class="from-addr">${escapeHtml(msg.from)}</div>
          <div class="to-line">to <b>${escapeHtml(msg.to)}</b></div>
          ${msg.cc ? `<div class="to-line">cc <b>${escapeHtml(msg.cc)}</b></div>` : ""}
        </div>
        <div class="meta-date">${escapeHtml(formatLongDate(msg.date))}</div>
      </div>
      ${bodyHtml}
      ${msg.attachments.length ? `<div class="detail-section-title">${icon("paperclip")}${msg.attachments.length} attachment${msg.attachments.length > 1 ? "s" : ""}</div><ul class="attachment-list">${attachmentsHtml}</ul>` : ""}
    </div>
    <div class="detail-actions">
      ${actionButton("reply-btn", "reply", "Reply", "r")}
      ${actionButton("forward-btn", "forward", "Forward", "f")}
      ${folder === "DRAFTS" ? actionButton("send-draft-btn", "send", "Send", "s") : ""}
      ${moveAction}
      ${deleteAction}
    </div>
  `;
  document.getElementById("reply-btn").addEventListener("click", () => openCompose(replyPrefill(currentMessage)));
  document.getElementById("forward-btn").addEventListener("click", () => openCompose(forwardPrefill(currentMessage)));
  document.getElementById("send-draft-btn")?.addEventListener("click", () => openCompose(draftPrefill(currentMessage), { account, uid }));
  document.getElementById("archive-btn")?.addEventListener("click", () =>
    moveCurrentMessage("archive_message", "ARCHIVE", { inProgress: "Archiving...", done: "Archived" })
  );
  document.getElementById("unarchive-btn")?.addEventListener("click", () =>
    confirmAndMove("Unarchive this message?", "unarchive_message", "INBOX", { inProgress: "Unarchiving...", done: "Unarchived" })
  );
  document.getElementById("restore-btn")?.addEventListener("click", () =>
    confirmAndMove("Restore this message to Inbox?", "restore_message", "INBOX", { inProgress: "Restoring...", done: "Restored" })
  );
  document.getElementById("delete-forever-btn")?.addEventListener("click", () =>
    confirmAndMove("Permanently delete this message? This cannot be undone.", "permanently_delete_message", null, {
      inProgress: "Deleting...",
      done: "Deleted",
    })
  );
  document.getElementById("delete-btn")?.addEventListener("click", () =>
    confirmAndMove("Delete this message?", "delete_message", "TRASH", { inProgress: "Deleting...", done: "Deleted" })
  );
  if (msg.body_html) renderHtmlBody(document.getElementById("message-body-frame"), msg.body_html);
  messageDetail.querySelectorAll("li.attachment-item").forEach((li) => {
    const filename = li.dataset.filename;
    const contentType = li.dataset.contentType;
    if (!isImageType(contentType)) return;
    invoke("get_attachment_data", { account, folder, uid, filename })
      .then((base64) => {
        const img = document.createElement("img");
        img.className = "attachment-thumb";
        img.src = `data:${contentType};base64,${base64}`;
        li.querySelector(".attachment-preview").replaceChildren(img);
      })
      .catch((err) => console.error(err));
  });
  messageDetail.querySelectorAll("button[data-filename]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const filename = btn.dataset.filename;
      const dest = await save_file_dialog(filename);
      if (!dest) return;
      try {
        await invoke("save_attachment", { account, folder, uid, filename, destPath: dest });
      } catch (err) {
        alert(String(err));
      }
    });
  });
}

function replyPrefill(msg) {
  const subject = /^re:/i.test(msg.subject) ? msg.subject : `Re: ${msg.subject}`;
  const quoted = msg.body
    .split("\n")
    .map((line) => `> ${line}`)
    .join("\n");
  return {
    to: msg.from,
    subject,
    body: `\n\n${quoted}`,
  };
}

function forwardPrefill(msg) {
  const subject = /^fwd:/i.test(msg.subject) ? msg.subject : `Fwd: ${msg.subject}`;
  const header = `---------- Forwarded message ----------\nFrom: ${msg.from}\nDate: ${formatLocalDate(msg.date)}\nSubject: ${msg.subject}\nTo: ${msg.to}`;
  return {
    to: "",
    subject,
    body: `\n\n${header}\n\n${msg.body}`,
  };
}

function draftPrefill(msg) {
  return {
    to: msg.to,
    cc: msg.cc,
    subject: msg.subject,
    body: msg.body,
  };
}

async function markMessageRead(account, folder, uid) {
  // Only skip the round trip when we positively know it's already read —
  // the message may not be in the cache at all (e.g. a stale on-disk cache
  // that a background refresh hasn't replaced yet).
  const cached = folderCache[account]?.[folder]?.find((m) => m.uid === uid);
  if (cached && !cached.unread) return;

  // Optimistic: the row stops looking unread immediately; the server catches up.
  if (cached) {
    cached.unread = false;
    persistFolderCache(account, folder);
  }
  renderAccountSwitcher();
  if (isViewing(account, folder)) messageList.querySelector(`.message-item[data-uid="${uid}"]`)?.classList.remove("unread");
  try {
    await invoke("mark_read", { account, folder, uid });
  } catch (err) {
    console.error(err);
  }
}

async function checkForNewMail(account) {
  let messages;
  try {
    messages = await invoke("list_messages", { account, folder: "INBOX" });
  } catch (err) {
    console.error(err);
    return;
  }
  if (!accounts.some((a) => a.username === account)) return; // removed meanwhile
  folders(account).INBOX = messages;
  persistFolderCache(account, "INBOX");
  if (isViewing(account, "INBOX")) renderMessageList();
  else renderAccountSwitcher();

  const unread = messages.filter((m) => m.unread);
  const known = knownUnreadUids[account];
  if (known) {
    const fresh = unread.filter((m) => !known.has(m.uid));
    fresh.forEach((m) => notifyNewMail(account, m));
    prefetch(account, "INBOX", fresh.slice(0, PREFETCH_TOP).map((m) => m.uid));
  }
  knownUnreadUids[account] = new Set(unread.map((m) => m.uid));
}

async function notifyNewMail(account, message) {
  try {
    const { isPermissionGranted, requestPermission, sendNotification } = window.__TAURI__.notification;
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (!granted) return;
    const subject = message.subject || "(no subject)";
    sendNotification({ title: message.from, body: accounts.length > 1 ? `${subject}\n→ ${account}` : subject });
  } catch (err) {
    console.error(err);
  }
}

async function save_file_dialog(defaultName) {
  const { save } = window.__TAURI__.dialog;
  return await save({ defaultPath: defaultName });
}

// ---- Compose ----------------------------------------------------------------

const composeModal = document.getElementById("compose-modal");
const composeForm = document.getElementById("compose-form");
const composeFrom = document.getElementById("compose-from");
const attachmentListEl = document.getElementById("attachment-list");

// Set while editing an existing draft (via the Send button on a DRAFTS
// message) to { account, uid }, so submitCompose knows to remove the
// superseded draft afterward instead of leaving a stale duplicate behind.
// null for a plain new message.
let editingDraft = null;

function openCompose(prefill = {}, draft = null) {
  editingDraft = draft;
  attachmentPaths = [];
  attachmentListEl.innerHTML = "";
  composeForm.reset();
  composeFrom.innerHTML = usableAccounts()
    .map((a) => `<option value="${escapeHtml(a.username)}">${escapeHtml(a.username)}</option>`)
    .join("");
  composeFrom.value = draft?.account ?? currentAccount;
  if (prefill.to) composeForm.elements.to.value = prefill.to;
  if (prefill.cc) composeForm.elements.cc.value = prefill.cc;
  if (prefill.subject) composeForm.elements.subject.value = prefill.subject;
  if (prefill.body) composeForm.elements.body.value = prefill.body;
  composeModal.classList.remove("hidden");
  composeForm.elements[prefill.to ? "body" : "to"].focus();
  if (prefill.to) composeForm.elements.body.setSelectionRange(0, 0);
}

function closeCompose() {
  composeModal.classList.add("hidden");
}

document.getElementById("compose-btn").addEventListener("click", () => openCompose());
document.getElementById("cancel-compose-btn").addEventListener("click", closeCompose);

document.getElementById("attachment-picker").addEventListener("click", async () => {
  const selected = await open({ multiple: true });
  if (!selected) return;
  const paths = Array.isArray(selected) ? selected : [selected];
  for (const p of paths) {
    attachmentPaths.push(p);
    const filename = p.split(/[\\/]/).pop();
    const li = document.createElement("li");
    li.className = "attachment-item";
    li.title = filename;
    li.innerHTML = `<span class="attachment-preview">${icon("file")}</span><div class="attachment-meta"><div class="attachment-name"></div></div>`;
    li.querySelector(".attachment-name").textContent = filename;
    attachmentListEl.appendChild(li);

    if (isImageType(guessImageType(filename))) {
      invoke("read_file_base64", { path: p })
        .then((base64) => {
          const img = document.createElement("img");
          img.className = "attachment-thumb";
          img.src = `data:${guessImageType(filename)};base64,${base64}`;
          li.querySelector(".attachment-preview").replaceChildren(img);
        })
        .catch((err) => console.error(err));
    }
  }
});

function guessImageType(filename) {
  const ext = filename.split(".").pop().toLowerCase();
  const types = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", bmp: "image/bmp", svg: "image/svg+xml" };
  return types[ext] || "";
}

// Closes the modal immediately and lets send/save-draft run in the
// background instead of blocking the UI on the SMTP/IMAP round trip;
// progress and outcome surface via the bottom-left status toast.
function submitCompose(shouldSend) {
  const form = new FormData(composeForm);
  const account = form.get("from");
  const payload = {
    account,
    to: form.get("to"),
    cc: form.get("cc") || "",
    subject: form.get("subject") || "",
    body: form.get("body") || "",
    attachmentPaths: [...attachmentPaths],
  };
  // Bcc only ever goes to send_email — save_draft has nowhere safe to persist
  // it (a draft has no envelope, and writing it into a header would defeat
  // the point of Bcc once the draft is later sent or read back).
  if (shouldSend) payload.bcc = form.get("bcc") || "";
  const supersededDraft = editingDraft;
  editingDraft = null;
  closeCompose();
  const inProgress = shouldSend ? "Sending..." : "Saving draft...";
  const done = shouldSend ? "Sent" : "Draft saved";
  showToast(inProgress);
  invoke(shouldSend ? "send_email" : "save_draft", payload)
    .then(() => {
      showToast(done, { autoHideMs: 2000 });
      invalidateFolder(account, shouldSend ? "SENT" : "DRAFTS");
      // A fresh draft/sent message was just appended above — remove the one
      // this replaced instead of leaving a stale duplicate sitting in Drafts.
      if (supersededDraft) {
        invoke("permanently_delete_message", { account: supersededDraft.account, folder: "DRAFTS", uid: supersededDraft.uid })
          .then(() => invalidateFolder(supersededDraft.account, "DRAFTS"))
          .catch((err) => console.error(err));
      }
    })
    .catch((err) => showToast(`Failed: ${err}`, { autoHideMs: 4000 }));
}

composeForm.addEventListener("submit", (e) => {
  e.preventDefault();
  submitCompose(true);
});

document.getElementById("save-draft-btn").addEventListener("click", () => {
  submitCompose(false);
});

// ---- Keyboard shortcuts -----------------------------------------------------

function isTypingTarget(el) {
  return el instanceof HTMLElement && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
}

document.addEventListener("keydown", (e) => {
  const mod = e.metaKey || e.ctrlKey;
  if (e.key === "Escape") {
    if (!accountMenu.classList.contains("hidden")) toggleAccountMenu(false);
    else if (!accountsModal.classList.contains("hidden")) accountsModal.classList.add("hidden");
    else if (document.activeElement === searchInput) {
      searchInput.value = "";
      searchQuery = "";
      searchInput.blur();
      renderMessageList();
    }
    return;
  }
  if (mod && e.key === "Enter" && !composeModal.classList.contains("hidden")) {
    e.preventDefault();
    composeForm.requestSubmit();
    return;
  }
  if (appView.classList.contains("hidden") || !composeModal.classList.contains("hidden") || !accountsModal.classList.contains("hidden")) return;

  if (mod && e.key.toLowerCase() === "k") {
    e.preventDefault();
    searchInput.focus();
    searchInput.select();
    return;
  }
  if (mod && /^[1-9]$/.test(e.key)) {
    const target = accounts[Number(e.key) - 1];
    if (target) {
      e.preventDefault();
      switchAccount(target.username);
    }
    return;
  }
  if (mod || e.altKey || isTypingTarget(e.target)) return;

  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (key === "c") openCompose();
  else if (key === "j" || key === "ArrowDown") moveSelection(1);
  else if (key === "k" || key === "ArrowUp") moveSelection(-1);
  else if (key === "/") searchInput.focus();
  else {
    const btn = messageDetail.querySelector(`.detail-actions [data-key="${CSS.escape(key)}"]`);
    if (!btn) return;
    btn.click();
  }
  e.preventDefault();
});

// ---- Rendering helpers ------------------------------------------------------

function isImageType(contentType) {
  return /^image\//.test(contentType || "");
}

function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str ?? "";
  return div.innerHTML.replace(/"/g, "&quot;");
}

// Links in mail content must go through the OS opener: the webview ignores
// target="_blank" and would otherwise navigate (or do nothing).
function openExternal(href) {
  if (/^(https?:|mailto:)/i.test(href || "")) {
    window.__TAURI__.opener.openUrl(href).catch((err) => console.error(err));
  }
}

messageDetail.addEventListener("click", (e) => {
  const a = e.target.closest("a[href]");
  if (!a) return;
  e.preventDefault();
  openExternal(a.getAttribute("href"));
});

// HTML mail bodies come pre-sanitized (scripts/handlers stripped server-side)
// but still carry the sender's own CSS, so they're rendered in a sandboxed
// iframe to keep that styling from leaking into the app UI. No allow-scripts
// means embedded/inline JS can never execute regardless of sanitization.
// allow-same-origin only lets this parent frame read the iframe's DOM to size
// it and intercept link clicks — safe to combine with no-scripts.
function renderHtmlBody(iframe, html) {
  const css = getComputedStyle(document.documentElement);
  const [bg, color, font, size] = ["--bg", "--text", "--font-reading", "--reading-size"].map((v) => css.getPropertyValue(v).trim());
  const fontsHref = new URL("fonts/fonts.css", document.baseURI).href;
  const wrapped = `<!doctype html><html><head><base target="_blank"><meta charset="utf-8">
    <link rel="stylesheet" href="${fontsHref}">
    <style>
      html, body { height: auto !important; min-height: 0 !important; overflow: visible !important; }
      body { margin: 0; font-family: ${font}; font-size: ${size}; line-height: 1.6; color: ${color}; background: ${bg}; word-wrap: break-word; }
      /* Marketing emails often size their outer wrapper for a fixed preview pane
         (height/max-height + overflow:hidden); left alone that clips the real
         content to a sliver of the message. Force any direct child of body to
         size to its content instead. */
      body > * { height: auto !important; max-height: none !important; overflow: visible !important; }
      img { max-width: 100%; }
    </style>
    </head><body>${html}</body></html>`;
  iframe.addEventListener("load", () => {
    const doc = iframe.contentDocument;
    if (!doc) return;
    const resize = () => {
      iframe.style.height = `${doc.documentElement.scrollHeight}px`;
    };
    resize();
    new ResizeObserver(resize).observe(doc.documentElement);
    doc.querySelectorAll("img").forEach((img) => img.addEventListener("load", resize));
    doc.addEventListener("click", (e) => {
      const a = e.target.closest?.("a[href]");
      if (!a) return;
      e.preventDefault();
      openExternal(a.getAttribute("href"));
    });
  });
  iframe.srcdoc = wrapped;
}

// Renders plain-text/markdown-ish email bodies as safe HTML: escape first, then
// only ever insert markup we generated ourselves (never raw user/email content).
// Sentinel uses Unicode Private-Use-Area code points, which can't appear in
// normal text, so stashed placeholders can never collide with real content.
const PLACEHOLDER_START = "\uE000";
const PLACEHOLDER_END = "\uE001";

function formatBody(raw) {
  let text = escapeHtml(raw);
  const placeholders = [];
  const stash = (html) => {
    placeholders.push(html);
    return `${PLACEHOLDER_START}${placeholders.length - 1}${PLACEHOLDER_END}`;
  };

  // markdown links: [label](https://... | mailto:...)
  text = text.replace(/\[([^\]\n]+)\]\((https?:\/\/[^\s)]+|mailto:[^\s)]+)\)/g, (_, label, url) =>
    stash(`<a href="${url}" target="_blank" rel="noopener noreferrer">${label}</a>`)
  );
  // bare urls
  text = text.replace(/https?:\/\/[^\s<]+/g, (url) =>
    stash(`<a href="${url}" target="_blank" rel="noopener noreferrer">${url}</a>`)
  );

  text = text
    .replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*\n]+)\*(?!\*)/g, "$1<em>$2</em>")
    .replace(/`([^`\n]+)`/g, "<code>$1</code>");

  text = text.replace(
    new RegExp(`${PLACEHOLDER_START}(\\d+)${PLACEHOLDER_END}`, "g"),
    (_, i) => placeholders[Number(i)]
  );
  return text;
}

init();
