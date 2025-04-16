---
summary: Support for MUC vCards and avatars
labels:
- 'Stage-Stable'
...

# Introduction

This module adds the ability to set vCard for MUC rooms. One of the most common use case is to be able to define an avatar for your own MUC room.

# Usage

Add "vcard\_muc" to your modules\_enabled list:

``` {.lua}
Component "conference.example.org" "muc"
modules_enabled = {
  "vcard_muc",
}
```

# Compatibility

  ------------------------- ----------
  trunk^[as of 2024-10-22]  Works
  0.12                      Works
  ------------------------- ----------

