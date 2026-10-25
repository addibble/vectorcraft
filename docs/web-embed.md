# Embedding the web build

A page on the same site can run VectorCraft's web build (`apps/vectorcraft-web`) in an `<iframe>`, hand it a file to open, and store what it saves instead of letting the browser download it. A file manager's "Edit in VectorCraft" (such as a Nextcloud app) is the intended use. The page talks to VectorCraft with `postMessage`. Code: `apps/vectorcraft-web/src/host.rs` (the bridge) and `src/protocol.rs` (its rules, tested natively).

## Turning it on

Load the web build with `?host=parent` in a frame on the **same origin** as the page:

```html
<iframe id="vectorcraft" src="/vectorcraft/index.html?host=parent"></iframe>
```

Without `?host=parent`, or when the page isn't framed, nothing changes: no listener is installed and saves download as usual. A page with an opaque origin (a sandboxed frame without `allow-same-origin`, a `file:` page) gets no bridge either.

What changes in host mode:

- Everything VectorCraft would download goes to the parent as `vectorcraft:save` instead: Save, Save As, Save a Copy and the exports. There is still no save panel, so the parent decides where each file goes.
- The parent is told when unsaved changes appear or go away (`vectorcraft:dirty`). VectorCraft doesn't ask before the page is left; guarding that is the parent's job.
- With `saveOnClose` (see `vectorcraft:configure`), closing a modified document saves it to the parent instead of asking Save Changes, and it closes once the parent says the save was stored (`vectorcraft:saved`).
- With `&recovery=parent` as well (`?host=parent&recovery=parent`), Data Recovery keeps its copies with the parent instead of the browser's local storage. See [Data Recovery kept by the page](#data-recovery-kept-by-the-page).

## Messages

Every message is a plain object with a string `type`. VectorCraft posts only to `window.parent`, with its own origin as the target origin.

| `type` | Direction | Fields | Meaning |
|---|---|---|---|
| `vectorcraft:ready` | to the parent | `version` (string) | The app is running and listening. Send the file now. |
| `vectorcraft:open` | from the parent | `name` (string), `bytes` (`ArrayBuffer` or `Uint8Array`), `title` (string, optional) | Open this file, as if it were picked with File › Open; its tab shows `title`. |
| `vectorcraft:save` | to the parent | `id` (number), `name` (string), `bytes` (`ArrayBuffer`, transferred) | Store this file. |
| `vectorcraft:saved` | from the parent | `id` (number), `name` (string), `ok` (boolean), `error` (string, optional) | Save `id` was stored (`ok: true`), or couldn't be. |
| `vectorcraft:dirty` | to the parent | `dirty` (boolean) | Unsaved changes appeared (`true`) or went away (`false`). |
| `vectorcraft:configure` | from the parent | `hide`, `hideMenus` (arrays of strings), `hideMark`, `saveOnClose`, `saveRecoveredInPlace` (booleans), `emptyMessage`, `emptyLink` (strings) | Hide commands, menus and the brand mark the host has no use for, say what the window shows with no document open, save on closing instead of asking, and save a recovered copy under its own name. |
| `vectorcraft:emptyLink` | to the parent | none | The user clicked the empty window's link (`emptyLink`). |
| `vectorcraft:recoveryWrite` | to the parent | `name` (string), `bytes` (`ArrayBuffer`, transferred) | With `&recovery=parent`: keep Data Recovery entry `name`, replacing any of that name. |
| `vectorcraft:recoveryRemove` | to the parent | `name` (string) | With `&recovery=parent`: delete entry `name` (one that isn't there is no error). |
| `vectorcraft:recoveryEntries` | from the parent | `entries` (array of `{name, bytes}`) | With `&recovery=parent`: the entries the parent keeps; recovery copies a crash left among them are offered. |
| `vectorcraft:failed` | to the parent | `error` (string) | Before `ready`: the app couldn't start. After `ready`: the last `vectorcraft:open`, `vectorcraft:configure`, `vectorcraft:saved` or `vectorcraft:recoveryEntries` was refused; the app keeps running. |

**`vectorcraft:ready`** comes once, when the app has started. `version` is the app's version (`CARGO_PKG_VERSION`). A host that has heard neither `ready` nor `failed` after a reasonable time should assume the app couldn't start. The commonest reason is a browser without WebGPU or WebGL2.

**`vectorcraft:open`** needs both fields:

- `name` is the file's name, such as `Logo.svg`. Its extension decides how the file is read, as for a picked file, so anything File › Open reads can be sent. It also becomes the document's name (its path, as a file opened from disk has), which saves are named after and a recovery copy remembers, a native `.vectorcraft` document's too, which would otherwise be known by the title saved inside it.
- `bytes` is the file's contents. The buffer may be transferred.
- `title` (optional, up to 200 characters) is what the document's tab shows in place of `name`, such as a project's name `Poster` for a file the host names `Poster v5.vectorcraft`. Without it the tab shows the file name with its extension, as it does for any file. The title is the tab's alone: the document keeps `name`, which saves (`vectorcraft:save`), exports, Document Info, printing and the questions about the file go by, so the host's own names come back to it unchanged. Once the document's name changes (Save As) its tab shows the new name. A title is given per file, since only the host knows which part of its name the user should see; there is no setting to drop extensions from every tab.
- A message without a non-empty `name`, without `bytes` of one of those two types, or with a `title` that isn't such a string is refused: VectorCraft answers `vectorcraft:failed` and opens nothing.
- A file that has both but can't be read (an empty or damaged file) is reported inside the app, like any file that fails to open.

**`vectorcraft:save`**: `name` is a bare file name with no folders, the name a download would have had. `id` numbers the saves since the app started (1, 2, 3, …), so an answer can say which one it means. The parent stores the file and reports any failure to the user itself. VectorCraft treats the save as done, as it does a download: after Save or Save As the document counts as saved, and `dirty: false` follows.

**`vectorcraft:saved`** answers a `vectorcraft:save`: `id` and `name` are the save's own, `ok` says whether the parent stored it, and `error` (optional, up to 1,000 characters) says why not, in the user's language. VectorCraft acts only on the answer a close is waiting for (see `saveOnClose`); answering every save is allowed, and the others change nothing. A message without a whole-number `id`, a non-empty `name` and a boolean `ok` is refused with `vectorcraft:failed`.

**`vectorcraft:configure`** fits the app to a host that manages files itself. Every field is optional, and each message replaces the last one whole, so a field left out (or `null`) is cleared:

- `hide` lists command ids the user isn't offered, such as `file.new`, `file.open` or `file.print`. A string ending in `*` hides every id it starts (`file.openRecent*`). A hidden command leaves the menus (and `ui.menu.list`), the command palette and the buttons that run it, such as the Home button for `app.home`, and its shortcut does nothing. The host's own control requests (`window.vectorcraft.execute`) still run it. Ids are the commands' own (`engine.commands` lists them); an id no command has is ignored. At most 1,000 ids of up to 200 characters each.
- `hideMenus` lists menus to leave out of the menu bar, by their English titles (`File`, `Help`), whatever the user's language. Their commands still run by shortcut and from the command palette unless `hide` lists them too, so a host that hides `File` keeps Save on its shortcut. At most 1,000 titles of up to 200 characters each.
- `hideMark` set to `true` hides the brand mark at the left of the application bar. The Discord button at its right follows `help.discord` in `hide`, as the Home button follows `app.home`.
- `emptyMessage` is what the window says while no document is open, such as "No file open.", in place of the Home screen. It is shown as given, so the host sends it in the user's language. Up to 1,000 characters; blank means none.
- `emptyLink` is a link shown after the message, such as "Open one from the Files app". VectorCraft follows nothing itself: a click posts `vectorcraft:emptyLink` to the parent, which takes the user where the link says. Up to 1,000 characters; blank means none.
- `saveOnClose` set to `true` replaces the Save Changes question for a host that stores every save itself. See [Closing without the Save Changes question](#closing-without-the-save-changes-question).
- `saveRecoveredInPlace` set to `true` has Save write a copy Data Recovery restored under its own name (the name the host opened it under), as Save writes any document, a `vectorcraft:save` like any other. Without it a restored copy is saved as, the save options asking for a name with its old one suggested, since its file may have changed since it was copied: right where VectorCraft writes the file itself, but not for a host that stores every save and merges it with what changed since. Save As still asks.

A message of any other shape (`hide` or `hideMenus` not an array of non-empty strings, `hideMark`, `saveOnClose` or `saveRecoveredInPlace` not a boolean, `emptyMessage` or `emptyLink` not a string, or any of them too long) is refused with `vectorcraft:failed`, and nothing changes. Fields it doesn't name are ignored, so a newer host's configuration works on an older build, which leaves out only what it doesn't know. Send it after `ready`, and again whenever the host's needs change.

### Closing without the Save Changes question

Without `saveOnClose`, closing a modified document (File › Close, a tab's ×, Close All, Quit) asks Save / Don't Save / Cancel, and Save posts a `vectorcraft:save` that VectorCraft counts as done. A host whose store can fail would lose the edits if the document closed then. With `saveOnClose: true`:

1. Closing a modified document saves it as File › Save does: a `vectorcraft:save` with its `id`. The document stays open, and the status line says it closes once the page has stored it. (A save that asks something first, such as replacing a file that opening it lost parts of, asks as usual and closes nothing.)
2. The parent stores the file and answers `vectorcraft:saved` with the save's `id` and `name`.
3. `ok: true`: the document closes, and a Close All or Quit goes on to the next modified document, saving each the same way, one at a time. If the user changed the document after the save, it stays open, still modified.
4. `ok: false`: the document stays open and counts as modified again (`dirty: true` follows); the status line shows `error`, and a Close All or Quit stops there. No answer at all leaves the document open just the same: there is no timeout.

While a close waits, the document can't be closed again (nor can Close All or Quit run), but other documents with nothing unsaved close as usual. If the user saves the document again meanwhile, the close waits for the answer to that newer save, since it holds more. Documents with nothing unsaved close at once, as before.

The host itself can still close a document and throw its changes away without any question, such as when it replaces the document with a newer copy: the command `file.close` with `discard: true` (`window.vectorcraft.execute("file.close", {index, discard: true})`, `index` defaulting to the active document) closes it unsaved, with or without `saveOnClose`, ending any close or question waiting for it. No menu item or shortcut passes `discard`, so the user is never offered it. A build without this answers the command with an error (an unknown parameter), and the host can fall back to answering the Save Changes question with Don't Save (`ui.dialog.set {field: "discard", value: true}`, then `ui.dialog.confirm`).

### Data Recovery kept by the page

On the web Data Recovery (Preferences › File Handling › Data Recovery) keeps its copies of modified documents in the browser's local storage, which holds a few megabytes per site: a large document's copy doesn't fit, and the status line says "Couldn't save recovery data". A host that stores files itself can keep the copies instead, wherever it likes (its own server, say, so they outlast the browser): load the build with `?host=parent&recovery=parent`.

1. Every change to the copies is posted: `vectorcraft:recoveryWrite` creates or replaces an entry and `vectorcraft:recoveryRemove` deletes one. The parent keeps each entry's bytes under its `name` exactly. A write that can't be posted fails the recovery save, which the status line reports; the parent reports its own failures to store one.
2. Names are `<area>/<entry>`: each running app keeps its entries in an area of its own (`1793284512-1`), and an entry is a copy's document (`<copy>.vectorcraft`), its details (`<copy>.json`, JSON: the document's `title` and `path`, the name it was opened under, its `images` and when it was written), an image the area's copies share (`img-….blob`, named by its content, so it never changes), or the area's `heartbeat` (Unix seconds, as text, refreshed every minute while the app runs). The parent may read the details, to keep each copy with the file it is of, say; the rest is VectorCraft's own.
3. A page that frames VectorCraft later gives the entries back, after `ready`: `vectorcraft:recoveryEntries` with `entries`, each `{name, bytes}` (bytes an `ArrayBuffer` or `Uint8Array`). They join the entries held (a later message adds to them; an entry of a name already held is replaced), and VectorCraft offers the copies among them that a crash left behind, with the Recover Documents question, as the desktop app does at launch. The copies of an app that still runs (its heartbeat fresh, or its tab holding its Web Lock in this browser) are never offered, so entries from other pages and other devices may be given too. Restoring a copy moves it into this app's area, and discarding deletes it: both are posted as above.
4. VectorCraft holds the entries in memory while it runs, so a recovery save, a restore and the question answer at once without waiting for the parent; the entries it holds are only those it wrote and was given.

At most 100,000 entries per message, each name up to 300 characters with a single `/` between non-blank parts. A message of any other shape, or one sent without `&recovery=parent`, is refused with `vectorcraft:failed`, and none of it is taken. Without `&recovery=parent` nothing changes: the copies stay in local storage and the recovery messages are never sent.

**`vectorcraft:dirty`** is sent after the first frame and then whenever the answer changes, so the first one is usually `dirty: false`. `true` means at least one open document has unsaved changes.

## Who is heard

The bridge reads a message only when all of these hold:

1. it comes from `window.parent` (`event.source`), not from another window or frame on the site;
2. its origin (`event.origin`) is exactly the page's own: same scheme, host and port;
3. its `type` is `vectorcraft:open`, `vectorcraft:configure`, `vectorcraft:saved` or `vectorcraft:recoveryEntries`.

Anything else is ignored without a reply, so a page on another origin can neither open files in VectorCraft nor learn that it is there. Saves and other replies go only to `window.parent`, with the page's own origin as the target, so the browser delivers them to no other site. A same-origin parent can already script the frame directly, so the bridge gives it nothing it couldn't otherwise do; it only spares it reaching into the app's internals.

## Compatibility

Hosts should ignore message types and fields they don't know; VectorCraft does the same. `ready`'s `version` lets a host tell which build it is talking to.

## Example host

A minimal page (error handling left out) that opens a file it fetched and writes saves back next to it:

```js
const frame = document.getElementById('vectorcraft')
window.addEventListener('message', async (event) => {
  if (event.origin !== window.location.origin || event.source !== frame.contentWindow) return
  const message = event.data
  switch (message.type) {
    case 'vectorcraft:ready': {
      const bytes = await (await fetch('/files/Logo.svg')).arrayBuffer()
      frame.contentWindow.postMessage({ type: 'vectorcraft:configure', hide: ['file.new', 'file.newDialog', 'file.open', 'file.openRecent*', 'app.home'], emptyMessage: 'No file open.', emptyLink: 'Open one from the Files app', saveOnClose: true }, window.location.origin)
      frame.contentWindow.postMessage({ type: 'vectorcraft:open', name: 'Logo.svg', bytes }, window.location.origin, [bytes])
      break
    }
    case 'vectorcraft:emptyLink':
      window.location.assign('/files')
      break
    case 'vectorcraft:save': {
      const stored = await fetch(`/files/${encodeURIComponent(message.name)}`, { method: 'PUT', body: message.bytes })
      // Needed only with saveOnClose: a close waits for this.
      frame.contentWindow.postMessage({ type: 'vectorcraft:saved', id: message.id, name: message.name, ok: stored.ok, error: stored.ok ? undefined : 'Not saved: the server said ' + stored.status }, window.location.origin)
      break
    }
    case 'vectorcraft:dirty':
      unsaved = message.dirty
      break
    case 'vectorcraft:failed':
      console.error('VectorCraft:', message.error)
      break
  }
})
let unsaved = false
window.addEventListener('beforeunload', (event) => { if (unsaved) event.preventDefault() })
```
