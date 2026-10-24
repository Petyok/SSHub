# Stage 2 protocol half (WP2a): results

Branch `poc/protocol`. Code: `src/sync/protocol.rs`, `src/sync/ratchet.rs`, `src/sync/enroll.rs` with their
`tests` modules. All numbers below come from `cargo test --lib sync::` on a Linux x86_64 build container
(debug profile); `sync::protocol` 15 tests and `sync::enroll` 8 tests pass.

## Round (issue P1, convergence)

Two or more in-process stores run `run_round` against `run_sync_serve` over an in-memory frame pipe.

| Case | Test | Result |
|---|---|---|
| honest, two stores, first contact then ratchet | `two_stores_converge_and_the_ratchet_advances_every_round` | state hashes equal; pair key equal on both sides; epoch 0, 1, 2 |
| honest, relay through a third device | `a_third_device_learns_through_a_relay_and_all_converge` | all three hashes equal |
| long tail (2300 ops, more than `MAX_OPS_PER_TAIL`) | `a_long_tail_travels_in_several_frames` | applied 2300, hashes equal |
| reordered and duplicated tails | `reordered_and_duplicated_tails_still_converge` | applied 6, duplicates 6, hashes equal |
| malformed ops (not JSON, empty, trailing byte) | `malformed_input_is_an_error_and_changes_nothing` | `Err`, store unchanged, no pair committed |
| caps (tail over 2000 ops, heads over 256) | `caps_are_enforced_per_message` | `Err` naming the cap |
| garbage frames, wrong message for the state | `garbage_frames_and_wrong_order_end_the_round` | `Err` (malformed frame or protocol violation) |
| forged signature | `a_forged_op_is_counted_as_rejected` | applied 0, rejected 1 |
| ops of a device the receiver does not know | `ops_of_a_device_only_the_peer_knows_are_refused_without_harm` | rejected, rest applied, no panic |
| equivocation between two peers | `equivocation_is_proven_between_peers_and_forwarded_to_a_third` | both sides list the forger in `forks` |
| fork proof forwarded to a device that never saw the fork | same test | the third device verifies the evidence itself and marks the device compromised; a compromised device is then refused |
| version mismatch, unknown peer, impersonation | `version_unknown_peer_and_impersonation_are_refused` | all refused with a reason |

Design points that the tests pin down:

* The server speaks first; both sides sign the hello transcript (possession of the device key).
* Receiver sorts ops per chain and retries order-only failures (unknown author, gap) after the rest.
* `Heads` carries chain heads, the sender's fresh self-signed head, up to 64 gossiped signed heads and
  up to 16 fork proofs. Fork proofs are processed after the pulls so the chains they refer to are present.
  Unsigned head claims that contradict ours without proof mark the device `suspect` (ask it directly).
* Requested ranges are checked: a tail may only contain ops of the requested device above `after_seq`.
* Per-message caps: `MAX_OPS_PER_TAIL`, `MAX_HEADS` 256, `MAX_GOSSIP_HEADS` 64, `MAX_FORK_PROOFS` 16,
  `MAX_WANTS_PER_PHASE` 1024, `MAX_OPS_PER_ROUND` 200000; a `Tail` frame is cut at half the frame cap.

## Ratchet and transcript MAC

The pair key `K(0)` comes from the first authenticated exchange (or from an enrollment). Every round proves
the current epoch with a MAC, every frame goes into one transcript, and `Finish` MACs the transcript.

* Clone one round behind: `a_clone_that_missed_one_round_is_behind_and_moves_nothing`. The original and
  the peer advance; a disk copy of the original (same identity, older pair state) is reported `behind` by
  both sides and no op moves.
* Whichever copy syncs first wins: `whichever_copy_syncs_first_wins_and_the_other_is_behind`. The first copy
  to arrive is accepted; the original then looks stale and has to be re-enrolled (SAS).
* Lost acknowledgement: `a_lost_acknowledgement_does_not_strand_an_honest_pair`. The server commits only
  after the client's acknowledgement and keeps `next` until then; the next round settles (epochs 1/0, then 2/2).
* Modified frame: `any_modified_frame_fails_the_round_and_commits_nothing` adds one inert JSON field to each
  frame in turn, client side and server side (every frame except the final acknowledgement). Every case is
  noticed (an `Err`, or `behind` when it hits the handshake) and no pair state advances past what the peer
  can verify.

Residuals (by construction): a clone that is exactly one round behind is accepted when the last
acknowledgement was lost (indistinguishable from the honest client that crashed). The client keeps
`K(epoch-1)` for the same reason, so a server clone one round behind is accepted by a client in that
window. The first contact of a pair is trust on first use; the SAS enrollment is the authenticated start.

## SAS enrollment (issue P1, MITM harness)

Both sides commit (BLAKE3) to identity keys, ephemeral key, nonce and name before either reveals; the SAS
is four BIP39 words (44 bits) from the hash of both reveals; each side signs the transcript with its device
key; the pair key is derived from the ephemeral exchange.

| Harness | N | Result |
|---|---|---|
| man in the middle substituting keys, through the full protocol, committing before it sees either reveal | 300 | 0 matching SAS |
| attacker that sees the responder's reveal after committing and grinds 64 alternatives, then reveals the best | 200 | refused 200 of 200 ("does not open the commitment") |
| independent SAS draws (statistics) | 10000 pairs | 0 full matches; 3 first-word matches (expected about 4.9, uniform) |
| swapped identity key, renamed device, wrong role | unit | commitment check fails |
| declined comparison on either side | 2 | both sides `Err`, nobody registered, no pair state |
| silent peer | 1 | times out |

An attacker that wants matching words has to find keys for both legs after seeing the other leg's reveal,
but each reveal is bound to a commitment sent before the other reveal exists, so each attempt is one
independent 44-bit draw (probability about 5.7e-14 per attempt).

## Not run

* The `Conn` adapter: `Conn` is not on `poc/tui` yet, so rounds are generic over the `SyncLink` trait
  (`send(&Value)`, `recv() -> Value`, `recv_first()`); `impl SyncLink for Conn` is a three-line adapter.
* Pruned and checkpoint messages exist on the wire and `Pruned` is handled on receipt, but the sending
  side waits for the prune floor and checkpoint store API.
* Real SSH between two machines.
