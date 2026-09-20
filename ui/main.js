const { invoke } = window.__TAURI__.core;
const { open, confirm } = window.__TAURI__.dialog;

let currentFolder = "INBOX";
let currentMessage = null; // { folder, uid, from, subject, body }
let attachmentPaths = [];
const folderCache = {}; // folder -> messages[], avoids refetching on every nav click

const FOLDER_TITLES = { INBOX: "Inbox", SENT: "Sent", DRAFTS: "Drafts", ARCHIVE: "Archive", TRASH: "Trash" };

const setupView = document.getElementById("setup-view");
const appView = document.getElementById("app-view");
const setupError = document.getElementById("setup-error");
const messageList = document.getElementById("message-list");
const messageDetail = document.getElementById("message-detail");
const folderTitle = document.getElementById("folder-title");
const refreshBtn = document.getElementById("refresh-btn");
const inboxUnreadBadge = document.getElementById("inbox-unread-badge");

let knownUnreadUids = null; // null until the first INBOX poll establishes a baseline
const POLL_INTERVAL_MS = 60000;

async function showApp() {
  setupView.classList.add("hidden");
  appView.classList.remove("hidden");
  await loadFolder(currentFolder);
  await checkForNewMail();
  setInterval(checkForNewMail, POLL_INTERVAL_MS);
}

async function init() {
  try {
    const account = await invoke("get_account");
    if (!account) return;
    if (await invoke("is_expired")) {
      prefillSetupForm(account);
      setupError.textContent = "Your session expired — please re-enter your password.";
      return;
    }
    await showApp();
  } catch (e) {
    console.error(e);
  }
}

function prefillSetupForm(account) {
  const form = document.getElementById("account-form");
  form.elements.username.value = account.username;
  form.elements.imap_host.value = account.imap_host;
  form.elements.imap_port.value = account.imap_port;
  form.elements.smtp_host.value = account.smtp_host;
  form.elements.smtp_port.value = account.smtp_port;
  form.elements.sent_folder.value = account.sent_folder;
  form.elements.drafts_folder.value = account.drafts_folder;
  form.elements.archive_folder.value = account.archive_folder;
  form.elements.trash_folder.value = account.trash_folder;
}

document.getElementById("account-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = new FormData(e.target);
  const account = {
    username: form.get("username"),
    imap_host: form.get("imap_host"),
    imap_port: Number(form.get("imap_port")),
    smtp_host: form.get("smtp_host"),
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
    await showApp();
  } catch (err) {
    setupError.textContent = String(err);
  }
});

document.querySelectorAll(".folder-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".folder-btn").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    currentFolder = btn.dataset.folder;
    loadFolder(currentFolder);
  });
});

refreshBtn.addEventListener("click", () => loadFolder(currentFolder, { force: true }));

function renderMessageList(folder, messages) {
  folderTitle.textContent = FOLDER_TITLES[folder] ?? folder;
  if (folder === "INBOX") updateUnreadBadge(messages);

  if (messages.length === 0) {
    messageList.innerHTML = "<p class='placeholder'>No messages</p>";
    return;
  }
  messageList.innerHTML = "";
  for (const m of messages) {
    const item = document.createElement("div");
    item.className = m.unread ? "message-item unread" : "message-item";
    item.dataset.uid = String(m.uid);
    item.innerHTML = `${m.unread ? '<span class="unread-dot"></span>' : ""}<strong>${escapeHtml(m.from)}</strong><br/><span class="subject">${escapeHtml(m.subject)}</span><br/><span class="date">${escapeHtml(m.date)}</span>`;
    item.addEventListener("click", () => {
      messageList.querySelectorAll(".message-item").forEach((el) => el.classList.remove("active"));
      item.classList.add("active");
      showMessage(folder, m.uid);
    });
    messageList.appendChild(item);
  }
}

function updateUnreadBadge(inboxMessages) {
  const count = inboxMessages.filter((m) => m.unread).length;
  if (count === 0) {
    inboxUnreadBadge.classList.add("hidden");
  } else {
    inboxUnreadBadge.textContent = String(count);
    inboxUnreadBadge.classList.remove("hidden");
  }
}

async function loadFolder(folder, { force = false } = {}) {
  messageDetail.innerHTML = "<p class='placeholder'>Select a message</p>";
  currentMessage = null;

  if (!force && folderCache[folder]) {
    renderMessageList(folder, folderCache[folder]);
    return;
  }

  messageList.innerHTML = "<p class='placeholder'>Loading...</p>";
  refreshBtn.classList.add("spinning");
  try {
    const messages = await invoke("list_messages", { folder });
    folderCache[folder] = messages;
    renderMessageList(folder, messages);
  } catch (err) {
    messageList.innerHTML = `<p class="error">${escapeHtml(String(err))}</p>`;
  } finally {
    refreshBtn.classList.remove("spinning");
  }
}

function invalidateFolder(folder) {
  delete folderCache[folder];
  if (currentFolder === folder) loadFolder(folder, { force: true });
}

async function confirmAndMove(question, command, destFolder) {
  if (await confirm(question, { title: "tinymail", kind: "warning" })) {
    moveCurrentMessage(command, destFolder);
  }
}

async function moveCurrentMessage(command, destFolder) {
  if (!currentMessage) return;
  const { folder, uid } = currentMessage;
  try {
    await invoke(command, { folder, uid });
  } catch (err) {
    alert(String(err));
    return;
  }
  currentMessage = null;
  messageDetail.innerHTML = "<p class='placeholder'>Select a message</p>";

  // The message already left `folder` server-side, so just drop it from the
  // cached list in place instead of paying for a full IMAP refetch — a
  // network round trip we already know the answer to. `destFolder`'s cache
  // is still stale (fresh content, wrong position — see list ordering fix),
  // so that one does need a real refetch, but only lazily, next time it's opened.
  if (folderCache[folder]) {
    folderCache[folder] = folderCache[folder].filter((m) => m.uid !== uid);
    if (currentFolder === folder) renderMessageList(folder, folderCache[folder]);
    else if (folder === "INBOX") updateUnreadBadge(folderCache[folder]);
  }
  delete folderCache[destFolder];
}

async function showMessage(folder, uid) {
  messageDetail.innerHTML = "<p class='placeholder'>Loading...</p>";
  try {
    const msg = await invoke("get_message", { folder, uid });
    currentMessage = { folder, uid, from: msg.from, subject: msg.subject, body: msg.body };
    const bodyHtml = msg.body_html
      ? `<iframe id="message-body-frame" sandbox="allow-same-origin" title="Message body"></iframe>`
      : `<div class="body">${formatBody(msg.body)}</div>`;
    const attachmentsHtml = msg.attachments
      .map(
        (a) =>
          `<li class="attachment-item" data-filename="${escapeHtml(a.filename)}" data-content-type="${escapeHtml(a.content_type)}" title="${escapeHtml(a.filename)} (${a.size} bytes)">
            <span class="attachment-preview">${isImageType(a.content_type) ? "" : attachmentIcon(a.filename)}</span>
            <button data-filename="${escapeHtml(a.filename)}">Save</button>
          </li>`
      )
      .join("");
    messageDetail.innerHTML = `
      <h2>${escapeHtml(msg.subject)}</h2>
      <p><strong>From:</strong> ${escapeHtml(msg.from)}</p>
      <p><strong>To:</strong> ${escapeHtml(msg.to)}</p>
      <p><strong>Date:</strong> ${escapeHtml(msg.date)}</p>
      <div class="message-toolbar">
        <button id="reply-btn">Reply</button>
        ${
          folder === "ARCHIVE"
            ? `<button id="unarchive-btn">Unarchive</button>`
            : folder === "TRASH"
              ? `<button id="restore-btn">Restore</button>`
              : `<button id="archive-btn">Archive</button>`
        }
        ${folder !== "TRASH" ? `<button id="delete-btn">Delete</button>` : `<button id="delete-forever-btn">Delete Forever</button>`}
      </div>
      <hr/>
      ${bodyHtml}
      ${msg.attachments.length ? `<h3>Attachments</h3><ul class="attachment-list">${attachmentsHtml}</ul>` : ""}
    `;
    document.getElementById("reply-btn").addEventListener("click", () => openCompose(replyPrefill(currentMessage)));
    document.getElementById("archive-btn")?.addEventListener("click", () => moveCurrentMessage("archive_message", "ARCHIVE"));
    document.getElementById("unarchive-btn")?.addEventListener("click", () =>
      confirmAndMove("Unarchive this message?", "unarchive_message", "INBOX")
    );
    document.getElementById("restore-btn")?.addEventListener("click", () =>
      confirmAndMove("Restore this message to Inbox?", "restore_message", "INBOX")
    );
    document.getElementById("delete-forever-btn")?.addEventListener("click", () =>
      confirmAndMove("Permanently delete this message? This cannot be undone.", "permanently_delete_message")
    );
    document.getElementById("delete-btn")?.addEventListener("click", () =>
      confirmAndMove("Delete this message?", "delete_message", "TRASH")
    );
    markMessageRead(folder, uid);
    if (msg.body_html) renderHtmlBody(document.getElementById("message-body-frame"), msg.body_html);
    messageDetail.querySelectorAll("li.attachment-item").forEach((li) => {
      const filename = li.dataset.filename;
      const contentType = li.dataset.contentType;
      if (!isImageType(contentType)) return;
      invoke("get_attachment_data", { folder, uid, filename })
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
          await invoke("save_attachment", { folder, uid, filename, destPath: dest });
        } catch (err) {
          alert(String(err));
        }
      });
    });
  } catch (err) {
    messageDetail.innerHTML = `<p class="error">${escapeHtml(String(err))}</p>`;
  }
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

async function markMessageRead(folder, uid) {
  const cached = folderCache[folder]?.find((m) => m.uid === uid);
  if (!cached || !cached.unread) return; // already read, or list not cached — nothing to update

  try {
    await invoke("mark_read", { folder, uid });
  } catch (err) {
    console.error(err);
    return;
  }
  cached.unread = false;
  if (folder === "INBOX") updateUnreadBadge(folderCache.INBOX);
  const item = messageList.querySelector(`.message-item[data-uid="${uid}"]`);
  if (item) {
    item.classList.remove("unread");
    item.querySelector(".unread-dot")?.remove();
  }
}

async function checkForNewMail() {
  let messages;
  try {
    messages = await invoke("list_messages", { folder: "INBOX" });
  } catch (err) {
    console.error(err);
    return;
  }
  folderCache.INBOX = messages;
  if (currentFolder === "INBOX") renderMessageList("INBOX", messages);
  else updateUnreadBadge(messages);

  const unread = messages.filter((m) => m.unread);
  if (knownUnreadUids) {
    for (const m of unread) {
      if (!knownUnreadUids.has(m.uid)) notifyNewMail(m);
    }
  }
  knownUnreadUids = new Set(unread.map((m) => m.uid));
}

async function notifyNewMail(message) {
  try {
    const { isPermissionGranted, requestPermission, sendNotification } = window.__TAURI__.notification;
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (!granted) return;
    sendNotification({ title: message.from, body: message.subject || "(no subject)" });
  } catch (err) {
    console.error(err);
  }
}

async function save_file_dialog(defaultName) {
  const { save } = window.__TAURI__.dialog;
  return await save({ defaultPath: defaultName });
}

// Compose modal
const composeModal = document.getElementById("compose-modal");
const composeForm = document.getElementById("compose-form");
const composeError = document.getElementById("compose-error");
const attachmentListEl = document.getElementById("attachment-list");

function openCompose(prefill = {}) {
  attachmentPaths = [];
  attachmentListEl.innerHTML = "";
  composeForm.reset();
  composeError.textContent = "";
  if (prefill.to) composeForm.elements.to.value = prefill.to;
  if (prefill.subject) composeForm.elements.subject.value = prefill.subject;
  if (prefill.body) composeForm.elements.body.value = prefill.body;
  composeModal.classList.remove("hidden");
}

document.getElementById("compose-btn").addEventListener("click", () => openCompose());

document.getElementById("cancel-compose-btn").addEventListener("click", () => {
  composeModal.classList.add("hidden");
});

document.getElementById("attachment-picker").addEventListener("click", async () => {
  const selected = await open({ multiple: true });
  if (!selected) return;
  const paths = Array.isArray(selected) ? selected : [selected];
  for (const p of paths) {
    attachmentPaths.push(p);
    const filename = p.split("/").pop();
    const li = document.createElement("li");
    li.className = "attachment-item";
    li.title = filename;
    const preview = document.createElement("span");
    preview.className = "attachment-preview";
    preview.textContent = attachmentIcon();
    li.append(preview);
    attachmentListEl.appendChild(li);

    if (isImageType(guessImageType(filename))) {
      invoke("read_file_base64", { path: p })
        .then((base64) => {
          const img = document.createElement("img");
          img.className = "attachment-thumb";
          img.src = `data:${guessImageType(filename)};base64,${base64}`;
          preview.replaceChildren(img);
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

async function submitCompose(shouldSend) {
  const form = new FormData(composeForm);
  const payload = {
    to: form.get("to"),
    subject: form.get("subject") || "",
    body: form.get("body") || "",
    attachmentPaths,
  };
  composeError.textContent = "";
  try {
    await invoke(shouldSend ? "send_email" : "save_draft", payload);
    composeModal.classList.add("hidden");
    invalidateFolder(shouldSend ? "SENT" : "DRAFTS");
  } catch (err) {
    composeError.textContent = String(err);
  }
}

composeForm.addEventListener("submit", (e) => {
  e.preventDefault();
  submitCompose(true);
});

document.getElementById("save-draft-btn").addEventListener("click", () => {
  submitCompose(false);
});

function isImageType(contentType) {
  return /^image\//.test(contentType || "");
}

function attachmentIcon() {
  return "\u{1F4CE}"; // 📎
}

function escapeHtml(str) {
  const div = document.createElement("div");
  div.textContent = str ?? "";
  return div.innerHTML;
}

// HTML mail bodies come pre-sanitized (scripts/handlers stripped server-side)
// but still carry the sender's own CSS, so they're rendered in a sandboxed
// iframe to keep that styling from leaking into the app UI. No allow-scripts
// means embedded/inline JS can never execute regardless of sanitization.
// allow-same-origin only lets this parent frame read the iframe's DOM to size
// it and intercept link clicks — safe to combine with no-scripts.
function renderHtmlBody(iframe, html) {
  const wrapped = `<!doctype html><html><head><base target="_blank"><meta charset="utf-8">
    <style>
      html, body { height: auto !important; min-height: 0 !important; overflow: visible !important; }
      body { margin: 0; font-family: inherit; color: #fff; background: #1c1c1f; word-wrap: break-word; }
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
    doc.querySelectorAll("a[href]").forEach((a) => {
      a.addEventListener("click", (e) => {
        e.preventDefault();
        const href = a.getAttribute("href") || "";
        if (/^(https?:|mailto:)/i.test(href)) {
          window.__TAURI__.opener.openUrl(href).catch((err) => console.error(err));
        }
      });
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
