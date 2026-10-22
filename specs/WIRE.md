# The wire format of youarel

Every message is a byte string with one canonical form, so its digest and its signature cover exactly one meaning.
A reader MUST reject trailing bytes, an unknown tag, a length beyond the message, and a value outside its range.

## Primitive values

| Value | Form |
| --- | --- |
| `u8`, `u16`, `u32`, `u64` | Big-endian, of their fixed width |
| `bytes` | A `u32` length and the bytes |
| `text` | `bytes` that hold UTF-8 |
| `list<T>` | A `u32` count and the items in order |
| `option<T>` | A `u8` of zero for none, or one followed by the value |
| `digest` | 32 bytes of SHA-512/256 |

A list whose items have keys MUST hold them in strictly ascending order of their bytes.

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

The plaintext of a bundle is the state in the JSON form of `youarel-store`, and the plaintext of an entry is an admission.
A shortcut's note and a folder's mark travel in that form like their other fields, and a document without one reads as a state where no device wrote it.
