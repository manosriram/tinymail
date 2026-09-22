<img width="64" height="64" alt="tinymail" src="https://github.com/user-attachments/assets/c0b47a03-5be9-4df7-a276-f5767e8cef9c" />

tinymail - lightweight email client

tinymail is a small native desktop client for reading and sending email over IMAP and SMTP. It shows a single inbox alongside sent, drafts, archive, and trash, supports reading, composing, replying, forwarding, and attachments, and switches between light and dark themes.

The app connects to any IMAP and SMTP account through a local config file. There are no accounts or servers of its own, and no data leaves the machine except to talk directly to the mail server.

Update and add this yaml to `$HOME/tinymail.yml`
```yml
imap_host: IMAP_HOST
imap_port: 993
smtp_host: SMTP_HOST
smtp_port: 465
username: email
sent_folder: Sent
drafts_folder: Drafts
archive_folder: Archive
trash_folder: Trash
```

<img width="800" alt="tinymail inbox" src="assets/Screenshot 2026-09-22 at 11.20.06 PM.png" />

<img width="800" alt="tinymail message view" src="assets/Screenshot 2026-09-22 at 11.20.25 PM.png" />

<img width="800" alt="tinymail compose" src="assets/Screenshot 2026-09-22 at 11.20.33 PM.png" />
