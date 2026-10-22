# The wire format of youarel

Every message is a byte string with one canonical form, so its digest and its signature cover exactly one meaning.
A reader MUST reject trailing bytes, an unknown tag, a length beyond the message, and a value outside its range.

## Primitive values

| Value | Form |
| --- | --- |
| `u8`, `u16`, `u32`, `u64` | Big-endian, of their fixed width |
| `varint` | An unsigned integer of up to 64 bits in LEB128: seven bits a byte, the least significant first, and the high bit set on every byte but the last |
| `bytes` | A `u32` length and the bytes |
| `text` | `bytes` that hold UTF-8 |
| `list<T>` | A `u32` count and the items in order |
| `option<T>` | A `u8` of zero for none, or one followed by the value |
| `digest` | 32 bytes of SHA-512/256 |

A list whose items have keys MUST hold them in strictly ascending order of their bytes.
A reader MUST reject a `varint` longer than the shortest form of its value, and one beyond 64 bits.

## Keys and signatures

| Value | Form |
| --- | --- |
| `device_public` | The Ed25519 key (32 bytes), the ML-DSA-65 key (1952 bytes), and the X-Wing key (1216 bytes) |
| `signature` | The Ed25519 signature (64 bytes) and the ML-DSA-65 signature (3309 bytes) |
| `slot` | The device identifier, the X-Wing encapsulation (1120 bytes), and the sealed epoch key (48 bytes) |
| `sealed` | The key commitment (32 bytes), the nonce (24 bytes), and the ciphertext as `bytes` |
| `confirmation` | An HMAC-SHA-512 tag (64 bytes) |

## Messages

Each message starts with a four-byte magic and a `u16` version, which is 1.

| Magic | Message | Body |
| --- | --- | --- |
| `YRO1` | Operation | account `digest`, parents `list<digest>`, author `digest`, wall time `u64`, action, `signature` |
| `YRS1` | Segment | account `digest`, operations `list<bytes>`, sender `digest`, epoch `u32`, sequence `u64`, previous plaintext `digest`, kind, records `sealed`, `signature` |
| `YRT1` | Cursors | peers `list<(sender digest, sequence u64, plaintext digest)>` |
| `YRD1` | Delivered | account `digest`, operations `list<bytes>`, chains `list<(sender digest, published u64, incomplete u8, first u64, latest u64, snapshot option<u64>, segments list<bytes>)>`, more `u8` |
| `YRP1` | Chain head | first `u64`, latest `u64`, snapshot `option<u64>` |
| `YRJ1` | Join | invitation `u16`, joiner `device_public`, PAKE message `bytes` |
| `YRW1` | Welcome | invitation `u16`, PAKE message `bytes`, encapsulation (1120 bytes), inviter's `confirmation` |
| `YRC1` | Joined | invitation `u16`, joiner's `confirmation` |
| `YRE1` | Entry | invitation `u16`, admission `sealed` |
| `YRH1` | Admission | account `digest`, operations `list<bytes>`, epoch keys `list<(u32, 32 bytes)>`, snapshot hint `option<(sender digest, sequence u64)>` |
| `YRQ1` | Request | account `digest`, device `digest`, verb `u8`, time `u64`, nonce (16 bytes), payload `bytes`, `signature` |
| `YRM1` | Mail | a `u8` tag, then nothing for an opening, a nameplate `u16`, a side `u8`, and a message `bytes` for a drop, or a nameplate and a side for a pick or poll |
| `YRF1` | Locate | member `digest` |
| `YRG1` | Registration | account `digest`, members `list<digest>` |

A request's verb is 0 for a publication, 1 for a collection, 2 for an erasure, and 3 for an explicit delta bridge; a mail's side is 0 for the inviter and 1 for the joiner.
Publication and bridge payloads hold one segment, collection holds cursors, and erasure has no payload.
A request or mail payload is at most 64 MiB.
The relay answers an opening with the version and the nameplate, each a `u16`, and a question of location with the version and the account `digest`.
The relay's paths are `/v1/requests`, `/v1/mail`, and `/v1/locate`, which take a `POST` of one message under the media type `application/vnd.youarel.v1`, and answer with a status and a message, or with the reason of a refusal as text.

A segment, delivery, and admission list their operations in ascending order of the operations' digests, and an admission lists its epoch keys in ascending order of the epoch and then the key.
The genesis operation names the zero digest as its account, since the account's identifier is the genesis operation's digest.

An action is a `u8` tag and its fields:

| Tag | Action | Fields |
| --- | --- | --- |
| 0 | `Genesis` | `device_public` |
| 1 | `Add` | `device_public` |
| 2 | `Remove` | device `digest` |
| 3 | `Rotate` | epoch `u32`, commitment `digest`, slots `list<slot>` |
| 4 | `AddRecovery` | `device_public`, consent `signature` |

A signature covers the message from its magic up to the signature, under the label of its kind in [CRYPTO.md](CRYPTO.md).
An operation's digest covers the same bytes and the signature.
A recovery member's consent is its signature of the account `digest` under `youarel/v1/recovery-consent`.
A registration travels only between the relay's own parts and never reaches a device.

The plaintext of an entry is a metadata-only admission.
An admission carries no library records and is bounded so its ciphertext, including its authentication tag, fits the sealed-message limit.
Its optional snapshot hint names a positive sequence.
The retired `YRB1` bundle, `YRL1` collection, and `YRA1` admission formats are refused.
Membership operation bytes and the version remain unchanged.
A device reads a received state only as these records.

## Segment chains

A segment's sequence is positive.
Its kind is a `u8` of zero for a delta, or one followed by its snapshot part and total parts as `u32` values.
The part starts at zero, the total is positive, and the part MUST be less than the total.
The first and last sequence implied by the snapshot parts MUST be positive and fit in `u64`.
The predecessor of sequence one is the zero digest.
Each later segment names the preceding plaintext body's digest under `youarel/v1/segment`.

A segment's plaintext is one body of state records.
A body can contain a subset of the state's complete records; a snapshot distributes its records across parts without splitting an item.
The plaintext is at most 16 MiB, and the ordinary snapshot writer targets parts of at most 8 MiB.
A segment envelope reserves 4 KiB for fixed framing and at most 64 MiB for that allowance, the ciphertext, and its operation fields including their length prefixes.
Each operation is at most 2 MiB, and its list keeps the operation limits and digest order of the membership log.
The signature covers every segment field before the signature under `youarel/v1/segment`.

Cursors list at most 1,024 peers in strictly ascending device order, each with a positive sequence.
A chain head has positive first and latest sequences, with first at most latest.
Its optional snapshot start lies inside that range and names a complete retained snapshot.
A delivered page lists at most 64 chains in strictly ascending sender order, carries at most 1,024 segment envelopes, and sets more to zero or one.
The relay sends at most 64 envelopes per page and retains each chain's metadata even when its requested segments do not fit that page.
Its bound is 64 MiB plus 64 KiB of framing, counted with the same framing allowance and length prefixes by both the writer and reader.
The published time is the relay's milliseconds since the Unix epoch, renewed only by an authenticated publication or an exact replay of the latest envelope.
Incomplete is zero or one and reports whether the latest retained header is a snapshot part with later parts still pending.
It remains visible even after a reader applied every available segment, so a crashed writer cannot make an unfinished bootstrap appear complete.
Transport heads are availability hints; readers still authenticate each envelope and admit its records before advancing a cursor.

Mail tags are opening 0, drop 1, pick 2, and poll 3.
A pick can wait for a message; a poll returns immediately with one message or no content.

## The state's records

The records of a state are the magic `YRR1`, a `u16` version of 2, the writer table, the folders, the shortcuts, and the device names.
Version 1 lacks device names and is refused without changing its bytes.
Inside the records, a count, a length, and each part of a stamp are a `varint`, and a text is its length and its UTF-8 bytes.

| Part | Form |
| --- | --- |
| Writer table | A count and each clock writer identifier of 16 bytes, in strictly ascending order |
| Folders | A count and each folder's record, in strictly ascending order of identifiers |
| Shortcuts | A count and each shortcut's record, in strictly ascending order of identifiers |
| Device names | A count and each name register, in strictly ascending order of the full 32-byte membership identifiers |
| Base stamp | The wall time, the counter, and the device's place in the table |
| Modes | The two-bit mode of each register after the first, four to a `u8`, the first register in the lowest bits, and every bit past the last register clear |
| Own stamp | The zigzag form of the wall time minus the base's, wrapping, then the counter and the device's place |

| Record | Fields in order |
| --- | --- |
| Shortcut | Identifier (16 bytes), flags `u8`, base stamp (the link's), modes of the title, alias, folder, position, presence, and note (two bytes), each own stamp in the registers' order, link, title, alias when flagged, folder identifier when flagged, position, note when flagged |
| Folder | Identifier (16 bytes), flags `u8`, base stamp (the name's), modes of the mark, parent, position, and presence (one byte), each own stamp in the registers' order, name, mark when flagged, parent identifier when flagged, position |
| Device name | Membership identifier (32 bytes), base stamp, name text; no flags or modes |

A device name holds at most 128 UTF-8 bytes, has no leading or trailing whitespace, and contains no control characters.
An empty value clears the display name while retaining its stamped register.
Membership identities and clock writer identifiers occupy separate domains and never truncate into each other.

A record's flags are bit 0 for an item gone for good, bit 1 for a shortcut in a folder or a folder under a parent, bit 2 for a shortcut's alias or a folder's mark, bit 3 for a shortcut's note, and bit 4 for an item in the trash.
A folder never sets bit 3, no record sets bit 0 and bit 4 together, and no record sets a higher bit.
The presence register's value is the item in the trash with bit 4, gone for good with bit 0, and live with neither.
Records from before the trash wrote bit 0 for a removed item, which reads as gone for good, so their bytes keep their meaning.
A build from before the trash refuses bit 4 as a reserved flag, so it reads no state that holds an item in the trash.
Mode 0 takes the base stamp, mode 1 reads an own stamp, mode 2 names the stamp that no device wrote, which is wall time 0, counter 0, and the device of sixteen zero bytes, and mode 3 is reserved.

A position key is a text of one or more of the sixty-two symbols `0` to `9`, `A` to `Z`, and `a` to `z`, whose last symbol is not `0`, and position keys order as their bytes.
A reader requires only this form, whichever writer chose the key.
A writer reads a key that starts with a letter as an integer and a fraction.
The letters `a` to `z` take one to twenty-six symbols of integer after them, and `Z` down to `A` take one to twenty-six, so integers order as their keys.
A writer places a folder's first entry at `a1`, an entry after the last one at the least integer above it but never below `a1`, and an entry before the first one at the greatest integer below it but never above `Zz`.
It places an entry between two others at their midpoint, symbol by symbol, and at the midpoint past either end where no integer lies beyond the key.
It never writes an integer that ends in `0`, so every key it writes keeps the form above.

A reader MUST reject records that the writer would not have written:

- a reserved flag or mode;
- the flags of an item in the trash and of an item gone for good together;
- a flagged text that is empty;
- an own stamp that equals the base or the stamp that no device wrote;
- mode 2 when the base is the stamp that no device wrote, since mode 0 names it then;
- a counter beyond 32 bits, or a place beyond the table;
- a device in the table that no stamp names;
- a position that is no position key.

A device admits every link of the records again before it keeps them, as it admits a typed one.
The store keeps each item's registers in its database `library.redb` and writes these records from them for a publish.
