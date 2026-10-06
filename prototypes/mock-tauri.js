// Fake window.__TAURI__ so the real ui/ (index.html + main.js + styles.css)
// runs in a plain browser with sample mail, for comparing design prototypes.
// URL params: ?p=1..5 (theme, handled by serve.py), &theme=light|dark, &open=0 (no auto-open).
(() => {
  const params = new URLSearchParams(location.search);
  localStorage.clear();
  if (params.get("theme")) localStorage.setItem("tinymail-theme", params.get("theme"));

  const hoursAgo = (h) => new Date(Date.now() - h * 3600e3).toUTCString();
  const daysAgo = (d) => new Date(Date.now() - d * 86400e3).toUTCString();

  const accounts = [
    { username: "priya@northwind.io", imap_host: "imap.northwind.io", imap_port: 993, smtp_host: "smtp.northwind.io", smtp_port: 465, sent_folder: "Sent", drafts_folder: "Drafts", archive_folder: "Archive", trash_folder: "Trash", expired: false },
    { username: "priya.raman@gmail.com", imap_host: "imap.gmail.com", imap_port: 993, smtp_host: "smtp.gmail.com", smtp_port: 465, sent_folder: "Sent", drafts_folder: "Drafts", archive_folder: "Archive", trash_folder: "Trash", expired: false },
  ];

  const inbox = [
    { uid: 112, from: "daniel.okafor@northwind.io", subject: "Q4 planning — draft agenda for Thursday", date: hoursAgo(0.6), unread: true,
      body: `Hi Priya,\n\nAhead of Thursday's planning session, here's a first pass at the agenda. I've kept it to 90 minutes so we have room for discussion at the end.\n\n1. Q3 retrospective (15 min) — what shipped, what slipped, and why\n2. Customer feedback themes (20 min) — Maya will walk through the interview synthesis\n3. Roadmap candidates for Q4 (30 min)\n4. Staffing and dependencies (15 min)\n5. Open discussion (10 min)\n\nCould you take item 3? You have the clearest picture of the sync-engine work and the trade-offs against the reporting revamp.\n\nI've also put the shared doc here: https://docs.northwind.io/q4-planning\n\nLet me know if you'd like to reorder anything before I send the invite to the wider group.\n\nThanks,\nDaniel`,
      attachments: [{ filename: "Q3-retrospective.pdf", size: 482113, content_type: "application/pdf" }] },
    { uid: 111, from: "maya.chen@northwind.io", subject: "Interview synthesis is ready for review", date: hoursAgo(2.2), unread: true,
      body: `Hey Priya,\n\nThe synthesis from the 14 customer interviews is ready. Short version: offline reliability came up in 11 of them, far ahead of anything else.\n\nI'd love 20 minutes with you before Thursday to make sure the framing is right.\n\nMaya` },
    { uid: 110, from: "notifications@github.com", subject: "[northwind/sync-engine] Fix retry backoff on reconnect (#482)", date: hoursAgo(5), unread: false,
      body: `@lucas-m requested your review on this pull request.\n\nThe retry loop now caps backoff at 30 seconds and resets after a successful sync, instead of growing without bound.\n\nFiles changed: 4   +86 −23` },
    { uid: 109, from: "billing@figma.com", subject: "Your receipt for October", date: daysAgo(1), unread: false, html: true,
      body: "Receipt #FG-20931 — Organization plan, 12 seats." },
    { uid: 108, from: "lucas.martin@northwind.io", subject: "Re: On-call handoff notes", date: daysAgo(1.4), unread: false,
      body: `Thanks — all clear on my side. The only open item is the elevated latency alert in eu-west, which I've silenced until Monday's deploy.\n\nLucas` },
    { uid: 107, from: "hr@northwind.io", subject: "Reminder: benefits enrollment closes Friday", date: daysAgo(2), unread: false,
      body: `A reminder that open enrollment for next year's benefits closes this Friday at 5pm. If you don't make any changes, your current selections roll over.` },
    { uid: 106, from: "sara.lindqvist@acme-logistics.com", subject: "Contract renewal — revised terms attached", date: daysAgo(3), unread: false,
      body: `Hi Priya,\n\nPlease find the revised renewal terms attached. We've incorporated the changes to the support SLA we discussed last week.\n\nBest regards,\nSara Lindqvist\nAcme Logistics` },
    { uid: 105, from: "newsletter@stratechery.com", subject: "The economics of local-first software", date: daysAgo(4), unread: false,
      body: `This week: why sync is the hard part, and what that means for who wins.` },
    { uid: 104, from: "tom.becker@northwind.io", subject: "Lunch & learn: accessibility basics", date: daysAgo(6), unread: false,
      body: `We're running a 45-minute session on accessibility fundamentals next Wednesday. Pizza provided.` },
    { uid: 103, from: "security@northwind.io", subject: "Action required: rotate your API tokens", date: daysAgo(9), unread: false,
      body: `As part of our quarterly hygiene, please rotate any personal API tokens older than 90 days.` },
    { uid: 102, from: "events@saastr.com", subject: "Your ticket for SaaStr Europa", date: daysAgo(40), unread: false, body: `Your ticket is confirmed.` },
    { uid: 101, from: "ana.ruiz@northwind.io", subject: "Welcome to the team!", date: daysAgo(400), unread: false, body: `Welcome aboard, Priya!` },
  ];
  const sent = [
    { uid: 51, from: "priya@northwind.io", to: "daniel.okafor@northwind.io", subject: "Re: Roadmap review", date: daysAgo(1), unread: false, body: "Sounds good — see you Thursday." },
  ];
  const mailbox = { INBOX: inbox, SENT: sent, DRAFTS: [], ARCHIVE: [], TRASH: [{ uid: 7, from: "spam@example.com", subject: "You won!", date: daysAgo(2), unread: false, body: "No." }] };
  const gmailInbox = [
    { uid: 9, from: "mom@example.com", subject: "Sunday dinner?", date: hoursAgo(3), unread: true, body: "Are you coming over on Sunday?" },
  ];

  const receiptHtml = `<div style="font-family:Helvetica,Arial,sans-serif;max-width:560px;color:#222">
    <h2 style="margin:0 0 12px">Thanks for your payment</h2>
    <p>Receipt <b>#FG-20931</b> · Organization plan · 12 seats</p>
    <table style="width:100%;border-collapse:collapse;margin:16px 0">
      <tr><td style="padding:8px 0;border-bottom:1px solid #ddd">Organization plan × 12</td><td style="text-align:right;border-bottom:1px solid #ddd">$540.00</td></tr>
      <tr><td style="padding:8px 0">Total</td><td style="text-align:right"><b>$540.00</b></td></tr>
    </table>
    <p><a href="https://figma.com/billing">View billing settings</a></p></div>`;

  const find = (account, folder, uid) => (account === accounts[1].username ? gmailInbox : mailbox[folder] ?? []).find((m) => m.uid === uid);
  const delay = (ms) => new Promise((r) => setTimeout(r, ms));

  const handlers = {
    list_accounts: () => accounts,
    list_messages: ({ account, folder }) =>
      (account === accounts[1].username ? (folder === "INBOX" ? gmailInbox : []) : mailbox[folder] ?? []).map(({ uid, from, subject, date, unread }) => ({ uid, from, subject, date, unread })),
    get_message: ({ account, folder, uid }) => {
      const m = find(account, folder, uid);
      return { from: m.from, to: m.to ?? account, cc: m.cc ?? "", subject: m.subject, date: new Date(m.date).toISOString(), body: m.body, body_html: m.html ? receiptHtml : undefined, attachments: m.attachments ?? [] };
    },
    open_link: ({ url }) => { (window.__opened ??= []).push(url); },
    empty_trash: ({ account }) => { mailbox.TRASH.length = 0; },
    mark_read: ({ account, folder, uid }) => { const m = find(account, folder, uid); if (m) m.unread = false; },
  };

  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => { await delay(40); return handlers[cmd]?.(args); } },
    dialog: { open: async () => null, save: async () => null, confirm: async () => true },
    app: { getVersion: async () => "1.0.3" },
    window: { getCurrentWindow: () => ({ show: async () => {}, setFocus: async () => {} }) },
    notification: { isPermissionGranted: async () => false, requestPermission: async () => "denied", sendNotification: () => {} },
    opener: { openUrl: async (u) => { (window.__opened ??= []).push(u); } },
  };

  if (params.get("open") !== "0") {
    const tryOpen = () => {
      const item = document.querySelector(".message-item");
      if (item) item.click();
      else setTimeout(tryOpen, 50);
    };
    addEventListener("load", () => setTimeout(tryOpen, 100));
  }
})();
