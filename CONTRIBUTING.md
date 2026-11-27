# Contributing to DyoSnap

Thanks for helping. DyoSnap is a small hobby project, so please read this first.

## What fits

DyoSnap does two jobs, screenshots and recordings, and stays small. Changes that keep
it that way are welcome: bug fixes, better captures and recordings, editor tools, Linux desktop support, docs.

These will not be merged:

- Anything that makes taking a screenshot or recording more complicated, such as extra
  steps, clicks or dialogs. The goal of DyoSnap is to get both done in as few clicks as possible.
- Network access of any kind: uploads, accounts, telemetry, update checks, crash reports.
- Large new dependencies for small features.
- X11 support. DyoSnap on Linux is Wayland only.
- Code under GPL or other licenses that would stop the app from staying MIT
  (for example x264 in the macOS ffmpeg build).

## Issues

- **Bug:** say what you did, what happened and what you expected. Add your OS version,
  desktop (on Linux) and DyoSnap version (Preferences › About). Screenshots help.
- **Idea:** describe the problem first, then your solution.
- **Before a larger change**, open an issue so we can agree on the approach. That saves
  you work on a pull request that cannot be merged.

## Pull requests

1. Fork the repo and create a branch from `main`.
2. Build and run it as described in the [README](README.md#build-macos). On Linux, see
   [Linux](README.md#linux).
3. Keep the change focused: one fix or feature per pull request.
4. Run `scripts/check.sh` (clippy, tests, notices) and try your change in the running app.
   To run it before every push, turn on the hook once: `git config core.hooksPath .githooks`.
5. Describe what you changed and how you tested it. For UI changes, add a screenshot.

`main` is protected. All changes go through a pull request.

### Rules for code

- Linux code lives in `src/platform/linux/` behind `cfg(target_os = "linux")`. Keep
  platform code out of shared modules.
- New crates must use a license listed in [about.toml](about.toml). `scripts/notices.sh`
  fails otherwise.
- No local paths (like `/Users/<name>/...` or `/home/<name>/...`) in code, scripts or docs.
  `scripts/bundle.sh` checks the macOS build for them.
- Do not commit build output (`target/`, `dist/`, `build/`).
- Check the dependencies for known security issues with `cargo audit`.

## Signing (macOS)

`scripts/bundle.sh` signs the app with a certificate called **DyoSnap** if it is in
your keychain. Without it the app is signed ad hoc. That works too, but macOS then treats
every new build as a new app and asks for the screen-recording permission again.

The signature pins the exact certificate. macOS keeps the screen-recording permission only
as long as every build is signed with the same one. A new certificate, or a lost one, makes
every user grant the permission again. So give it a long life and keep a backup.

To create the certificate once (free, no Apple developer account):

1. Open **Keychain Access** (Spotlight: "Keychain Access", or `open -a "Keychain Access"`).
2. Menu **Keychain Access › Certificate Assistant › Create a Certificate…**
3. Name: `DyoSnap`, Identity Type: **Self-Signed Root**, Certificate Type: **Code Signing**.
   Tick **Let me override defaults**, then **Continue**.
4. Validity Period: `7300` days (20 years). Click **Continue** on the following pages and keep
   the defaults. Leave the email address empty, it would show in every signed app.
5. Keychain: **login**. Click **Create**.

The first build after that asks whether `codesign` may use the key: choose **Always Allow**.
To use a different certificate, set `DYOSNAP_SIGN_ID="<name>"`. `bundle.sh` warns 90 days
before the certificate expires and stops if two certificates have the same name.

**Backup:** In Keychain Access, under **My Certificates**, right-click **DyoSnap ›
Export**, save as `.p12` with a strong password, and keep the file and password in a password
manager. To restore it on another Mac, double-click the file.

## License

DyoSnap is under the [MIT License](LICENSE). By sending a pull request, you agree that
your contribution is published under the same license.

## No promises

This is a free-time project. Reviews may take a while, and not every pull request will be
merged. That is not a judgment of your work.
