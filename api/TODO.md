# To do

- Check when `require"core.modulemanager".load_modules_for_host(host)` might be necessary. Turns out some modules don’t load when added to `modules_enabled`.

- `GET /service-accounts/{username}/password (auth: prosody:admin)`
  - Not `PUT` to allow load balancing on the Pod API
