# Sync glossary: one plain word per design term

The design, the contract and the code keep their precise terms (ring, holder, SAS, head, Red). A person using the
terminal, the command line, the phone app or the phone page never has to learn them. This file fixes ONE user word for
each, so the README, the man page, the terminal, the app and the page say the same thing. Where a screen needs the design
term (a details view, a developer doc), it follows the plain word in brackets: `full device (holder)`.

Rules for everyone who writes a user string:

- Use the **user word** in every sentence a person reads. Never use two words for one thing.
- The design term belongs in details views, `--help` text for experts, logs and the design and contract documents.
- No internal names (op, lamport, seq, epoch numbers, `HeadAttest`, `DeviceAdd`) on a screen a person decides on.
- Host ids and device ids are shown the same way everywhere a person compares them (see "Ids" below).

| Design term | User word | One-line meaning | Where shown |
|-------------|-----------|------------------|-------------|
| ring | **your devices** (the group of devices that sync) | the laptops, servers and phone that share your host list and credentials | README, Sync tab title and first run, app first run, `sshub sync-status` |
| ring secret | **ring secret** (on the printed kit) | the long secret on the kit sheet that lets a new computer join and open saved passwords | kit sheets, import prompts, README |
| kit | **kit** (two printed sheets) | the master phrase sheet and the ring secret sheet | first run, import prompts |
| master phrase | **master phrase** (six words) | the six words that, with the kit, open saved passwords in Vault mode | first run, unlock prompt |
| holder | **full device** | a device that holds a piece of every saved password and can approve things | device list, role prompts, README |
| mirror | **view-only copy** | a device that gets your host list and encrypted data but holds no piece of any password | device list, role prompts, README |
| attended | **has a person at the keyboard** | a laptop you sit at, as opposed to a server that runs on its own | device list, enrollment |
| unattended | **runs on its own** (a server) | a device nobody sits at; it never votes and never blocks an approval | device list, enrollment |
| SAS, four words, short authentication string | **the four check words** | four words both screens show; answer yes only if they are identical | enrollment on the terminal, the app and the page |
| head, `head #17`, chain head | **last change** (number of the newest change a device has signed) | where a device's own history stops; shown only in details | details view, `sshub sync-status` |
| head pending, restored phone | **this phone must read its history from a terminal first** | after restoring from the 24 words the phone does not sign until it has the newest list of its own changes | app home card, page home card |
| Red / Yellow / Green (tiers) | **big change / needs one approval / nothing to approve** | big: a new full device, a removal, a mode change, a trust session, a new phone, a bulk export; one approval: releasing one password, a foreign edit; nothing: reading the host list | review list, phone approval screens |
| proofs (the three proofs) | **the three approvals** | the phone, a second device with a person at the keyboard, and the paper phrase or the emailed code | review list, phone approval screens |
| third factor | **paper phrase or emailed code** | the approval that does not come from your devices | review list, prompts |
| paper phrase | **paper phrase** (24 words) | the 24 words written on paper; a key to everything, keep it offline | prompts, README |
| ceremony | **approval round** | the set of approvals a big change collects; it expires after 3 days | review list |
| trust session, certificate, delegation certificate, cert | **trust session** | the phone lets one computer open some hosts without asking each time, for a set number of days | trust menu, phone screens, README (never "certificate" or "delegation" on a screen) |
| scope | **which hosts** | the group or list of hosts a trust session covers | trust session prompts |
| term | **how many days** | how long a trust session lasts: 7, 30 or 90 | trust session prompts |
| budget (max new hosts per day) | **new hosts per day** | how many hosts the computer may open for the first time in a day | trust session prompts |
| short code, snapshot | **short code** | a 16 character code that opens one password for 7 days without the phone | phone approval screen, short codes list, README (never "snapshot") |
| enrollment ("Add my phone", "Enroll with a terminal", `sync-phone-enroll`) | **add the phone** (in the app: "Connect to a terminal"; CLI verb unchanged) | pairing the phone with a terminal, in steps with a step count | terminal menu, app, page, README |
| enrollment steps (commitment, reveal, SAS, confirm) | **step names a person understands** | 1 "Terminal code", 2 "Phone code", 3 "Terminal keys", 4 "Compare the words", 5 "Finish". The terminal, the app and the page all show 5 steps | app, page, terminal |
| commitment, reveal | **code** / **keys** | the first exchange of an enrollment; never shown by those names | page and app step names |
| epoch | **sharing round** (number only in details) | the version of the way passwords are split; it changes after a removal | details view, phone "update needed" screen |
| reshare | **update the shares** (phone: "Update this phone after a device was removed") | after a removal the passwords are split again among the remaining devices | app, terminal after a revoke |
| wrap key, wrap generation | **phone key generation** (details only) | the phone's key set; renewed after a removal | details view only |
| behind | **missed some changes** (last in sync N days ago) | a device that has been away for 14 days or more and needs approving back | device list |
| stale | **away too long** (last in sync N days ago) | a device that has been away for 30 days or more | device list |
| suspect | **needs a look** | a device whose history looks wrong but is not proven bad | device list |
| compromised | **COMPROMISED** (keep as is) | a device signed two different histories; the whole screen alerts | alert, device list |
| operational | **in sync** | a device that is up to date and trusted | device list |
| revoke | **remove a device** (revoke) | cut a device out; it keeps what it already downloaded | menus, README |
| Vault / Quorum | **Vault** / **Quorum** (keep) | Vault: kit plus master phrase opens passwords; Quorum: any two of your devices open them | credential mode screens, README |
| co-signer | **second signer** | the device that cosigns a mode change | mode prompts |
| exemption, master-phrase exemption | **master phrase exemption** | a way for a server to open passwords with the master phrase in Quorum mode; advanced | advanced menu |
| fork | **two histories** (fork) | a device signed two different histories; treated as compromised | alert |
| checkpoint, prune | **tidy up** (details only) | old changes are summarized to keep the log small | details only |
| courier | **hand over** (details only) | the phone's signed changes reach other devices when it touches any terminal | details only |
| op, operation | **change** | one signed change in the log | details view |
| sync-serve, round | **sync** | one exchange between two devices over ssh | everywhere |

## Ids

| Thing | How it is shown to a person |
|-------|-----------------------------|
| host id (credential id) | 8 groups of 4 hex digits, uppercase, single spaces: `0A0A 0A0A 0A0A 0A0A 0A0A 0A0A 0A0A 0A0A` |
| device id | 16 hex digits, lowercase, as typed into a command: `723ed85f0566df0c` (shown in groups of 4 on a phone screen where it is typed or compared) |
| change id (op id) on the command line | the first 8 hex digits of the change's hash; any 6 or more digits that identify one change are accepted |

## The revocation rule, in one sentence

> To remove a stolen or lost device you need **three computers or servers you control**, the one you remove included,
> together with the phone: two other full devices that are not the one you remove, and the phone, approve the removal.

(Code: revoking a full device needs at least two remaining full terminals to contribute. If you cannot reach that,
change the passwords of the hosts and remove the device's key from every host's `authorized_keys` by hand.)

## Enrollment step names (terminal, app and page)

| Step | Name | What happens |
|------|------|--------------|
| 1 of 5 | Terminal code | the phone reads the code the terminal shows |
| 2 of 5 | Phone code | the terminal reads the code the phone shows |
| 3 of 5 | Terminal keys | the phone reads the terminal's keys |
| 4 of 5 | Compare the words | both screens show four words; confirm only if they are identical |
| 5 of 5 | Finish | the terminal's confirmation reaches the phone and the phone is added |

## One computer has no copy

With one computer there is no backup of your host list: the phone and the paper hold pieces of passwords, not your hosts.
Add a second computer, or a view-only copy on a server at a permanent address, before you rely on sync. Without that, a lost
computer means a lost host list.
