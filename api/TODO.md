# To do

- Check when `require"core.modulemanager".load_modules_for_host(host)` might be necessary. Turns out some modules don’t load when added to `modules_enabled`.

- `GET /service-accounts/{username}/password (auth: prosody:admin)`
  - Not `PUT` to allow load balancing on the Pod API

- Test that a second “first admin” cannot be created

- Migrate Workspace vCard at startup

  ```rs
  // See [RFC 6473: vCard KIND:application](https://www.rfc-editor.org/rfc/rfc6473.html).
  kind: Some(vcard4::Kind::Application),
  ```

- Force rosters sync at startup
  
  ```rs
  /// NOTE: Rosters resynchronization (for teams) is an expensive operation
  ///   (O(n^2)), therefore the API debounces it. If a team member is added but
  ///   the API is restarted before the debounce timeout (e.g. in tests), rosters
  ///   become inconsistent. This forces a resynchronization at startup.
  ```
