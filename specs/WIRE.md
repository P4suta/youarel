# Stored records

The product exposes no synchronization endpoint or transport command.
The [retired wire specification](https://github.com/P4suta/youarel/blob/d0fbc7afcb58bed077e9be67a956d9202e96df55/specs/WIRE.md) records the former message formats.

The local database retains its existing format and record encoding.
[Record rules](../crates/youarel-core/src/record_rules.rs) define the encoding.
[Database code](../crates/youarel-store/src/database.rs) validates the stored format.
Existing bookmarks, collections, visits, and icons remain local.
