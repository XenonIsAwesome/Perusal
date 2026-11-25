# Flatpak trade-off

Perusal drives the host's `git` and the user's agent CLIs (claude, gemini, codex, opencode) so credential helpers,
SSH config and partial clones behave as users expect. Inside a Flatpak sandbox those are not visible, so the manifest
requests `--talk-name=org.freedesktop.Flatpak` and runs them through `flatpak-spawn --host`.

That permission lets the app run arbitrary commands on the host, which largely defeats the sandbox. Users who want
isolation should prefer the AppImage or the native deb/rpm packages (which have the same trust level but no pretence
of confinement). We ship the Flatpak anyway for discoverability and document this limit here.
