You can communicate with the running niri instance over an IPC socket.
Check `niri msg --help` for available commands.

The `--json` flag prints the response in JSON, rather than formatted.
For example, `niri msg --json outputs`.

> [!TIP]
> If you're getting parsing errors from `niri msg` after upgrading niri, make sure that you've restarted niri itself.
> You might be trying to run a newer `niri msg` against an older `niri` compositor.

### Event Stream

<sup>Since: 0.1.9</sup>

While most niri IPC requests return a single response, the event stream request will make niri continuously stream events into the IPC connection until it is closed.
This is useful for implementing various bars and indicators that update as soon as something happens, without continuous polling.

The event stream IPC is designed to give you the complete current state up-front, then follow up with updates to that state.
This way, your state can never "desync" from niri, and you don't need to make any other IPC information requests.

Where reasonable, event stream state updates are atomic, though this is not always the case.
For example, a window may end up with a workspace id for a workspace that had already been removed.
This can happen if the corresponding workspaces-changed event arrives before the corresponding window-changed event.

To get a taste of the events, run `niri msg event-stream`.
Though, this is more of a debug function than anything.
You can get raw events from `niri msg --json event-stream`, or by connecting to the niri socket and requesting an event stream manually.

You can find the full list of events along with documentation [here](https://niri-wm.github.io/niri/niri_ipc/enum.Event.html).

### Programmatic Access

`niri msg --json` is a thin wrapper over writing and reading to a socket.
When implementing more complex scripts and modules, you're encouraged to access the socket directly.

Connect to the UNIX domain socket located at `$NIRI_SOCKET` in the filesystem.
Write your request encoded in JSON on a single line, followed by a newline character, or by flushing and shutting down the write end of the connection.
Read the reply as JSON, also on a single line.

You can use `socat` to test communicating with niri directly:

```sh
$ socat STDIO "$NIRI_SOCKET"
"FocusedWindow"
{"Ok":{"FocusedWindow":{"id":12,"title":"t socat STDIO /run/u ~","app_id":"Alacritty","workspace_id":6,"is_focused":true}}}
```

The reply is an `Ok` or an `Err` wrapping the same JSON object as you get from `niri msg --json`.

<sup>Since: next release</sup>
For more complex requests, you can pass `--print-request` to `niri msg` to see how the request should be formatted:

```sh
$ niri msg --print-request action focus-workspace 2
{"Action":{"FocusWorkspace":{"reference":{"Index":2}}}}
```

This is the format that you should use for communicating with the niri socket directly.
You can also use `niri msg raw-request` to send a raw JSON request without `socat`:

```sh
$ echo '{"Action":{"FocusWorkspace":{"reference":{"Id":8}}}}' | niri msg raw-request
"Handled"
```

You can find all available requests and response types in the [niri-ipc sub-crate documentation](https://niri-wm.github.io/niri/niri_ipc/).

### Scratchpad

<sup>Since: next release</sup>

The [scratchpad actions](./Configuration:-Key-Bindings.md#move-window-to-scratchpad-scratchpad-show-remove-window-from-scratchpad) accept a window `--id`, and `scratchpad-show` also accepts an `--app-id` regular expression (but not both at once):

```sh
niri msg action move-window-to-scratchpad --id 42
niri msg action toggle-window-in-scratchpad --id 42
niri msg action scratchpad-show --app-id '^dropdown$'
```

Windows have two fields for the scratchpad state:

- `is_in_scratchpad`: the window is in the scratchpad, either hidden or shown.
- `is_scratchpad_hidden`: the window is hidden in the scratchpad.

Hidden windows have no `workspace_id`.
A window that is being dragged with the mouse has no `workspace_id` either, so check `is_scratchpad_hidden` rather than `workspace_id` to tell whether a window is hidden.

```sh
niri msg --json windows | jq '.[] | select(.is_in_scratchpad)'
```

Similarly, `toggle-window-sticky` accepts a window `--id`, and windows have an `is_sticky` field for [sticky windows](./Configuration:-Key-Bindings.md#toggle-window-sticky).

`scratchpad-show` only acts on windows that are already in the scratchpad.
For example, a drop-down terminal script needs to handle three cases: the terminal isn't running, it's running but not in the scratchpad, or it's in the scratchpad:

```bash
#!/usr/bin/env bash
w=$(niri msg --json windows | jq -c '[.[] | select(.app_id == "dropdown")][0] // empty')
if [ -z "$w" ]; then
    niri msg action spawn -- foot --app-id dropdown
elif [ "$(jq -r .is_in_scratchpad <<<"$w")" = true ]; then
    niri msg action scratchpad-show --id "$(jq -r .id <<<"$w")"
else
    niri msg action move-window-to-scratchpad --id "$(jq -r .id <<<"$w")"
fi
```

Note that pressing this quickly several times in a row may start more than one terminal.

### Backwards Compatibility

The JSON output *should* remain stable, as in:

- existing fields and enum variants should not be renamed
- non-optional existing fields should not be removed

However, new fields and enum variants will be added, so you should handle unknown fields or variants gracefully where reasonable.

The formatted/human-readable output (i.e. without `--json` flag) is **not** considered stable.
Please prefer the JSON output for scripts, since I reserve the right to make any changes to the human-readable output.

The `niri-ipc` sub-crate (like other niri sub-crates) is *not* API-stable in terms of the Rust semver; rather, it follows the version of niri itself.
In particular, new struct fields and enum variants will be added.
