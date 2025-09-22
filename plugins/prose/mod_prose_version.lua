-----------------------------------------------------------
-- mod_prose_version: Advertises the Prose Pod version via
-- a HTTP ReST API.
-- version 0.1
-----------------------------------------------------------
-- Copyright (C) 2025 Rémi Bardon <remi@remibardon.name>
--
-- This project is MIT licensed. Please see the LICENSE
-- file in the source package for more information.
-----------------------------------------------------------

local json = require "util.json";

local function read_version(file_name)
  local file_path = (CFG_SOURCEDIR or ".").."/prose.version.d/"..file_name;
  local version_file = io.open(file_path);
  local version = nil;
  if version_file then
    version = version_file:read("*a"):gsub("%s*$", "");
    version_file:close();
  else
    module:log("warn", "Prose version file not found at %s", file_path);
  end
  return version;
end

module:depends("http");
module:provides("http", {
  title = "Prose Pod version";
  route = {
    ["GET /"] = function (event)
      local tag = read_version("VERSION") or "unknown";
      local commit = read_version("COMMIT") or "";
      local commit_short = string.sub(commit, 1, 7);
      local build_timestamp = read_version("BUILD_TIMESTAMP") or "";
      local build_date = string.sub(build_timestamp, 1, string.len('2000-01-01'));

      event.response.headers.content_type = "application/json";
      return json.encode({
        version = tag.." ("..build_date..")";
        tag = tag;
        build_date = build_date;
        build_timestamp = build_timestamp;
        commit_short = commit_short;
        commit_long = commit;
      });
    end;
  };
});
