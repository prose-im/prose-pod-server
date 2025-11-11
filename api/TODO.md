# To do

- Test that a second “first admin” cannot be created

- Migrate Workspace vCard at startup

  ```rs
  // See [RFC 6473: vCard KIND:application](https://www.rfc-editor.org/rfc/rfc6473.html).
  kind: Some(vcard4::Kind::Application),
  ```

- It’d be really nice if we had a global Prosody-style pub/sub system to send an event when `AuthToken`s are `Drop`ped and have `ProsodyOAuth2` automatically revoke the token.

- CI

- Test SERVICE_UNAVAILABLE on SIGHUP if error

- OAuth 2.0 client conserved across restarts (even failed)
  - Save it in the static config if not there already
    (use https://crates.io/crates/toml_edit).

- Get rid of `AppConfig` and use scoped configs everywhere instead?
  - This way we can have the Prose Pod API write in the configuration file and
    reload sub-configurations. Also decouples things more and paves the way for
    modules/plugins.
