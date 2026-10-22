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
| `YRB1` | Bundle | account `digest`, operations `list<bytes>`, sender `digest`, epoch `u32`, state `sealed`, `signature` |
| `YRJ1` | Join | invitation `u16`, joiner `device_public`, PAKE message `bytes` |
| `YRW1` | Welcome | invitation `u16`, PAKE message `bytes`, encapsulation (1120 bytes), inviter's `confirmation` |
| `YRC1` | Joined | invitation `u16`, joiner's `confirmation` |
| `YRE1` | Entry | invitation `u16`, admission `sealed` |
| `YRA1` | Admission | account `digest`, operations `list<bytes>`, epoch keys `list<(u32, 32 bytes)>`, state `bytes` |
| `YRQ1` | Request | account `digest`, device `digest`, verb `u8`, time `u64`, nonce (16 bytes), payload `bytes`, `signature` |
| `YRL1` | Collected | slots `list<(device digest, time u64, bundle bytes)>` |
| `YRM1` | Mail | a `u8` tag, then nothing for an opening, a nameplate `u16`, a side `u8`, and a message `bytes` for a drop, or a nameplate and a side for a pick |
| `YRF1` | Locate | member `digest` |
| `YRG1` | Registration | account `digest`, members `list<digest>` |

A request's verb is 0 for a publication, 1 for a collection, and 2 for an erasure; a mail's side is 0 for the inviter and 1 for the joiner.
The relay answers an opening with the version and the nameplate, each a `u16`, and a question of location with the version and the account `digest`.
The relay's paths are `/v1/requests`, `/v1/mail`, and `/v1/locate`, which take a `POST` of one message under the media type `application/vnd.youarel.v1`, and answer with a status and a message, or with the reason of a refusal as text.

A bundle and an admission list their operations in ascending order of the operations' digests, and an admission lists its epoch keys in ascending order of the epoch and then the key.
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

The plaintext of a bundle is the state's records, and so is the state of an admission; the plaintext of an entry is an admission.
A device also reads a state in the JSON form of `youarel-store` that builds from before the records send, and drops the icons such a document names.

## The state's records

The records of a state are the magic `YRR1`, a `u16` version of 1, the device table, the folders, and the shortcuts.
Inside the records, a count, a length, and each part of a stamp are a `varint`, and a text is its length and its UTF-8 bytes.

| Part | Form |
| --- | --- |
| Device table | A count and each device identifier of 16 bytes, in strictly ascending order |
| Folders | A count and each folder's record, in strictly ascending order of identifiers |
| Shortcuts | A count and each shortcut's record, in strictly ascending order of identifiers |
| Base stamp | The wall time, the counter, and the device's place in the table |
| Modes | The two-bit mode of each register after the first, four to a `u8`, the first register in the lowest bits, and every bit past the last register clear |
| Own stamp | The zigzag form of the wall time minus the base's, wrapping, then the counter and the device's place |

| Record | Fields in order |
| --- | --- |
| Shortcut | Identifier (16 bytes), flags `u8`, base stamp (the link's), modes of the title, alias, folder, position, removal, and note (two bytes), each own stamp in the registers' order, link, title, alias when flagged, folder identifier when flagged, position, note when flagged |
| Folder | Identifier (16 bytes), flags `u8`, base stamp (the name's), modes of the mark, parent, position, and removal (one byte), each own stamp in the registers' order, name, mark when flagged, parent identifier when flagged, position |

A record's flags are bit 0 for a removed item, bit 1 for a shortcut in a folder or a folder under a parent, bit 2 for a shortcut's alias or a folder's mark, and bit 3 for a shortcut's note; a folder never sets bit 3, and no record sets a higher bit.
Mode 0 takes the base stamp, mode 1 reads an own stamp, mode 2 names the stamp that no device wrote, which is wall time 0, counter 0, and the device of sixteen zero bytes, and mode 3 is reserved.

A reader MUST reject records that the writer would not have written:

- a reserved flag or mode;
- a flagged text that is empty;
- an own stamp that equals the base or the stamp that no device wrote;
- mode 2 when the base is the stamp that no device wrote, since mode 0 names it then;
- a counter beyond 32 bits, or a place beyond the table;
- a device in the table that no stamp names;
- a position that is no position key.

A device admits every link of the records again before it keeps them, as it admits a typed one.
The store keeps the same records in its file `state.v2`.
