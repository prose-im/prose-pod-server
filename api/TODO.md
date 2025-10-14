# To do

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

- It’d be really nice if we had a global Prosody-style pub/sub system to send an event when `AuthToken`s are `Drop`ped and have `ProsodyOAuth2` automatically revoke the token.
