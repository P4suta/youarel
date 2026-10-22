// The cursor of `segment_rules::step`, with three writers and one-register records.
// `crates/youarel-cli/tests/segment_replay/model.rs` replays every delivery through the production step.
// Digests encode the complete bounded plaintext injectively; zero names the absent predecessor.
module segments_model {
  const CHECK_PREDECESSOR: bool

  type Body = int -> int
  type Cursor = { sequence: int, digest: int }
  type Head = { first: int, latest: int, snapshot: int }
  type Segment = { sender: int, sequence: int, previous: int, snapshot: bool, epoch: int, body: Body }
  type Decision = { cursor: Cursor, verdict: str, jump: int }
  type Receipt = { active: bool, receiver: int, before: Cursor, segment: Segment, head: Head, decision: Decision }

  pure val DEVICES = Set(1, 2, 3)
  pure val MOST = 4
  pure val EMPTY = DEVICES.mapBy(_ => 0)
  pure val START: Cursor = { sequence: 0, digest: 0 }
  pure val NO_HEAD: Head = { first: 1, latest: 0, snapshot: 0 }
  pure val NO_SEGMENT: Segment = { sender: 1, sequence: 1, previous: 0, snapshot: false, epoch: 0, body: EMPTY }
  pure val NO_DECISION: Decision = { cursor: START, verdict: "none", jump: 0 }
  pure val NO_RECEIPT: Receipt = { active: false, receiver: 1, before: START, segment: NO_SEGMENT, head: NO_HEAD, decision: NO_DECISION }

  pure def digest(body: Body): int = 1 + body.get(1) + 3 * body.get(2) + 9 * body.get(3)
  pure def merge(a: Body, b: Body): Body = DEVICES.mapBy(d => if (a.get(d) >= b.get(d)) a.get(d) else b.get(d))

  pure def decide(cursor: Cursor, segment: Segment, advertised: Head): Decision = {
    val expected = cursor.sequence + 1
    val usable = advertised.snapshot >= expected and advertised.snapshot >= advertised.first and advertised.snapshot <= segment.sequence
    val verdict =
      if (segment.sequence < cursor.sequence) "Old"
      else if (segment.sequence == cursor.sequence)
        if (digest(segment.body) == cursor.digest) "Reapply" else "Forked"
      else if (segment.sequence == expected)
        if (not(CHECK_PREDECESSOR) or segment.previous == cursor.digest) "Apply" else "Forked"
      else if (advertised.snapshot == segment.sequence and advertised.first <= segment.sequence and segment.snapshot) "Apply"
      else if (usable) "Jump"
      else if (advertised.first > expected) "Lost"
      else "Gap"
    {
      cursor: if (verdict == "Apply") { sequence: segment.sequence, digest: digest(segment.body) } else cursor,
      verdict: verdict,
      jump: if (verdict == "Jump") advertised.snapshot else 0,
    }
  }

  var writes: int -> int
  var views: int -> Body
  var cursors: int -> (int -> Cursor)
  var chains: int -> List[Segment]
  var heads: int -> Head
  var allocated: int -> int
  var committed: int -> int
  var watermarks: int -> int
  var pending: int -> Segment
  var members: Set[int]
  var epoch: int
  var receipt: Receipt
  var event: str

  action init = all {
    writes' = EMPTY,
    views' = DEVICES.mapBy(_ => EMPTY),
    cursors' = DEVICES.mapBy(_ => DEVICES.mapBy(_ => START)),
    chains' = DEVICES.mapBy(_ => []),
    heads' = DEVICES.mapBy(_ => NO_HEAD),
    allocated' = EMPTY,
    committed' = EMPTY,
    watermarks' = EMPTY,
    pending' = Map(),
    members' = DEVICES,
    epoch' = 0,
    receipt' = NO_RECEIPT,
    event' = "init",
  }

  action edit(writer: int): bool = all {
    members.contains(writer), writes.get(writer) < 2,
    writes' = writes.set(writer, writes.get(writer) + 1),
    views' = views.set(writer, views.get(writer).set(writer, writes.get(writer) + 1)),
    cursors' = cursors, chains' = chains, heads' = heads, allocated' = allocated,
    committed' = committed, watermarks' = watermarks, pending' = pending,
    members' = members, epoch' = epoch, receipt' = NO_RECEIPT, event' = "edited",
  }

  // Building reserves one immutable plaintext, including across a crash before local commit.
  action build(writer: int, snapshot: bool): bool = {
    val held = chains.get(writer)
    val sequence = allocated.get(writer) + 1
    val first = committed.get(writer) == 0
    val whole = first or snapshot
    val body = if (whole) views.get(writer) else EMPTY.set(writer, writes.get(writer))
    val previous = if (held.length() == 0) 0 else digest(held.nth(held.length() - 1).body)
    val segment: Segment = { sender: writer, sequence: sequence, previous: previous, snapshot: whole, epoch: epoch, body: body }
    all {
      members.contains(writer), sequence <= MOST, not(pending.keys().contains(writer)),
      whole or writes.get(writer) > watermarks.get(writer),
      writes' = writes, views' = views, cursors' = cursors,
      chains' = chains.set(writer, held.append(segment)), heads' = heads,
      allocated' = allocated.set(writer, sequence), committed' = committed, watermarks' = watermarks,
      pending' = pending.put(writer, segment), members' = members, epoch' = epoch,
      receipt' = NO_RECEIPT, event' = "built",
    }
  }

  // The head becomes visible before the local watermark is acknowledged.
  action expose(writer: int): bool = {
    val segment = pending.get(writer)
    val advertised = heads.get(writer)
    val first = if (segment.snapshot) segment.sequence else advertised.first
    val snapshot = if (segment.snapshot) segment.sequence else advertised.snapshot
    all {
      pending.keys().contains(writer), advertised.latest < segment.sequence,
      writes' = writes, views' = views, cursors' = cursors,
      chains' = chains.set(writer, chains.get(writer).select(s => s.sequence >= first)),
      heads' = heads.set(writer, { first: first, latest: segment.sequence, snapshot: snapshot }),
      allocated' = allocated, committed' = committed, watermarks' = watermarks, pending' = pending,
      members' = members, epoch' = epoch, receipt' = NO_RECEIPT,
      event' = if (segment.snapshot and committed.get(writer) > 0) "compacted" else "exposed",
    }
  }

  action commit(writer: int): bool = {
    val segment = pending.get(writer)
    all {
      pending.keys().contains(writer), heads.get(writer).latest == segment.sequence,
      writes' = writes, views' = views, cursors' = cursors, chains' = chains, heads' = heads,
      allocated' = allocated, committed' = committed.set(writer, segment.sequence),
      watermarks' = watermarks.set(writer, segment.body.get(writer)),
      pending' = pending.keys().exclude(Set(writer)).mapBy(d => pending.get(d)),
      members' = members, epoch' = epoch, receipt' = NO_RECEIPT, event' = "committed",
    }
  }

  action interrupt(writer: int): bool = all {
    pending.keys().contains(writer),
    writes' = writes, views' = views, cursors' = cursors, chains' = chains, heads' = heads,
    allocated' = allocated, committed' = committed, watermarks' = watermarks, pending' = pending,
    members' = members, epoch' = epoch, receipt' = NO_RECEIPT, event' = "crashed",
  }

  action receive(receiver: int, segment: Segment, advertised: Head): bool = {
    val before = cursors.get(receiver).get(segment.sender)
    val decision = decide(before, segment, advertised)
    all {
      receiver != segment.sender,
      segment.epoch == 0 or members.contains(receiver),
      writes' = writes,
      views' = if (decision.verdict == "Apply") views.set(receiver, merge(views.get(receiver), segment.body)) else views,
      cursors' = cursors.set(receiver, cursors.get(receiver).set(segment.sender, decision.cursor)),
      chains' = chains, heads' = heads, allocated' = allocated, committed' = committed,
      watermarks' = watermarks, pending' = pending, members' = members, epoch' = epoch,
      receipt' = { active: true, receiver: receiver, before: before, segment: segment, head: advertised, decision: decision },
      event' = "received",
    }
  }

  action deliver(sender: int, receiver: int): bool = {
    nondet segment = chains.get(sender).indices().oneOf()
    val offered = chains.get(sender).nth(segment)
    all {
      offered.sequence <= heads.get(sender).latest,
      receive(receiver, offered, heads.get(sender)),
    }
  }

  // A quota may retain only deltas; the missing snapshot must be reported as Lost.
  action forgetSnapshot(writer: int): bool = {
    val advertised = heads.get(writer)
    all {
      advertised.snapshot > 0, advertised.latest > advertised.snapshot, not(pending.keys().contains(writer)),
      writes' = writes, views' = views, cursors' = cursors,
      chains' = chains.set(writer, chains.get(writer).select(s => s.sequence > advertised.snapshot)),
      heads' = heads.set(writer, { first: advertised.snapshot + 1, latest: advertised.latest, snapshot: 0 }),
      allocated' = allocated, committed' = committed, watermarks' = watermarks, pending' = pending,
      members' = members, epoch' = epoch, receipt' = NO_RECEIPT, event' = "forgotten",
    }
  }

  action remove = all {
    epoch == 0,
    writes' = writes, views' = views, cursors' = cursors, chains' = chains, heads' = heads,
    allocated' = allocated, committed' = committed, watermarks' = watermarks, pending' = pending,
    members' = Set(1, 2), epoch' = 1, receipt' = NO_RECEIPT, event' = "rotated",
  }

  action forge = receive(2, NO_SEGMENT.with("previous", 99), NO_HEAD.with("latest", 1))

  action step = {
    nondet writer = DEVICES.oneOf()
    nondet receiver = DEVICES.oneOf()
    nondet snapshot = Set(false, true).oneOf()
    any {
      edit(writer), build(writer, snapshot), expose(writer), commit(writer), interrupt(writer),
      deliver(writer, receiver), forgetSnapshot(writer), remove, forge,
    }
  }

  val chained = not(receipt.active) or receipt.decision.verdict != "Apply" or {
    val offered = receipt.segment.sequence
    any {
      offered == receipt.before.sequence + 1 and receipt.segment.previous == receipt.before.digest,
      offered > receipt.before.sequence + 1 and receipt.segment.snapshot
        and receipt.head.snapshot == offered and receipt.head.first <= offered,
    }
  }
  val monotone = not(receipt.active) or receipt.decision.cursor.sequence >= receipt.before.sequence
  val reserved = DEVICES.forall(d => committed.get(d) <= allocated.get(d)
    and allocated.get(d) <= committed.get(d) + 1
    and (pending.keys().contains(d) == (allocated.get(d) > committed.get(d))))
  val bounded = DEVICES.forall(d => watermarks.get(d) <= writes.get(d)
    and DEVICES.forall(peer => views.get(peer).get(d) <= writes.get(d)))
  val safe = chained and monotone and reserved and bounded

  val applied = receipt.active and receipt.decision.verdict == "Apply"
  val reapplied = receipt.active and receipt.decision.verdict == "Reapply"
  val jumped = receipt.active and receipt.decision.verdict == "Jump"
  val lost = receipt.active and receipt.decision.verdict == "Lost"
  val compacted = event == "compacted"
}

module segments {
  import segments_model(CHECK_PREDECESSOR = true).*

  run replayTest = init.then(edit(1)).then(build(1, false)).then(expose(1))
    .then(interrupt(1)).then(deliver(1, 2)).then(deliver(1, 2))
    .then(all { assert(reapplied and safe), init })

  run forkTest = init.then(forge).then(all {
    assert(receipt.decision.verdict == "Forked" and safe), init,
  })

  run jumpAndLostTest = init.then(build(1, false)).then(expose(1)).then(commit(1))
    .then(build(1, true)).then(expose(1)).then(commit(1))
    .then(edit(1)).then(build(1, false)).then(expose(1)).then(commit(1))
    .then(receive(2, chains.get(1).nth(1), heads.get(1)))
    .then(all { assert(jumped and safe), forgetSnapshot(1) })
    .then(receive(2, chains.get(1).nth(0), heads.get(1)))
    .then(all { assert(lost and safe), init })
}

module segments_probe {
  import segments_model(CHECK_PREDECESSOR = false).*

  run unchainedTest = init.then(forge).then(all { assert(not(safe)), init })
}
