// The membership log of `crates/youarel-core/src/membership_rules.rs`, over three devices that write operations and pass on what they know.
// `crates/youarel-cli/tests/membership_replay.rs` runs every device's log of every trace state through `membership_rules::resolve`, in two orders, and compares the members and the epoch.
// Any device that knows the genesis may write, whether its own view lists it or not, and any device may pass its log to any other, so the model covers a removed device that writes on and a relay that forwards everything.
module membership_model {
  // Whether a removal keeps the last member, which the probe switches off.
  const KEEP_LAST: bool

  type Action = Genesis | Add(int) | Remove(int) | Rotate(int)
  // An operation names its digest by `id`, which the replay spells as the digest's first byte.
  type Operation = { id: int, parents: Set[int], author: int, change: Action }
  type View = { members: Set[int], epoch: int }
  // The replay as `resolve` runs it: the step each member joined at, the epoch, the placed operations, and the next step.
  type Replay = { joined: int -> int, epoch: int, placed: Set[int], index: int }

  pure val DEVICES = Set(1, 2, 3)
  pure val FOUNDER = 1
  // The digest of the genesis, which names the account.
  pure val GENESIS_ID = 1
  // The most operations the log grows to, the genesis among them.
  pure val MOST = 4
  // The place of a non-member in the replay, after every member.
  pure val STRANGER = MOST + 1

  pure val GENESIS_OPERATION: Operation = { id: GENESIS_ID, parents: Set(), author: FOUNDER, change: Genesis }

  pure def rank(replay: Replay, author: int): int =
    if (replay.joined.keys().contains(author)) replay.joined.get(author) else STRANGER

  // Whether `a` comes before `b` in the replay: the earlier member first, then the smaller digest.
  pure def before(replay: Replay, a: Operation, b: Operation): bool =
    rank(replay, a.author) < rank(replay, b.author)
      or (rank(replay, a.author) == rank(replay, b.author) and a.id < b.id)

  // The operation the replay takes next among those whose parents it placed, or none.
  pure def upcoming(replay: Replay, operations: Set[Operation]): Set[Operation] = {
    val ready = operations.filter(o => not(replay.placed.contains(o.id)) and o.parents.subseteq(replay.placed))
    ready.filter(o => ready.forall(other => other == o or before(replay, o, other)))
  }

  pure def without(joined: int -> int, device: int): int -> int =
    joined.keys().exclude(Set(device)).mapBy(member => joined.get(member))

  // One step of the replay, as `Members::apply` takes it.
  pure def apply(replay: Replay, o: Operation): Replay = {
    val members = replay.joined.keys()
    val member = members.contains(o.author)
    val placed = replay.with("placed", replay.placed.union(Set(o.id))).with("index", replay.index + 1)
    match o.change {
      | Genesis =>
          if (o.id == GENESIS_ID and o.parents == Set() and members == Set())
            placed.with("joined", replay.joined.put(o.author, replay.index))
          else placed
      | Add(device) =>
          if (member and not(members.contains(device)))
            placed.with("joined", replay.joined.put(device, replay.index))
          else placed
      | Remove(device) =>
          if (member and members.contains(device) and (not(KEEP_LAST) or members.size() > 1))
            placed.with("joined", without(replay.joined, device))
          else placed
      | Rotate(epoch) =>
          if (member and epoch > replay.epoch) placed.with("epoch", epoch) else placed
    }
  }

  pure def resolve(operations: Set[Operation]): View = {
    val start = { joined: Map(), epoch: 0, placed: Set(), index: 0 }
    val done = 1.to(MOST).fold(start, (replay, _) => {
      val chosen = upcoming(replay, operations)
      if (chosen == Set()) replay else chosen.fold(replay, (r, o) => apply(r, o))
    })
    { members: done.joined.keys(), epoch: done.epoch }
  }

  // Every operation written so far, the operations each device knows, and what each device resolves from them.
  var log: Set[Operation]
  var known: int -> Set[Operation]
  var views: int -> View

  action init = all {
    log' = Set(GENESIS_OPERATION),
    known' = DEVICES.mapBy(d => if (d == FOUNDER) Set(GENESIS_OPERATION) else Set()),
    views' = DEVICES.mapBy(d => if (d == FOUNDER) resolve(Set(GENESIS_OPERATION)) else resolve(Set())),
  }

  // A device that knows the genesis writes an operation after everything it knows.
  action write(author: int, a: Action): bool = {
    val o = { id: log.size() + 1, parents: known.get(author).map(k => k.id), author: author, change: a }
    val learned = known.get(author).union(Set(o))
    all {
      log.size() < MOST,
      known.get(author).exists(k => k.id == GENESIS_ID),
      log' = log.union(Set(o)),
      known' = known.set(author, learned),
      views' = views.set(author, resolve(learned)),
    }
  }

  // A device passes everything it knows to another, as a bundle or a shared folder carries it.
  action pass(sender: int, receiver: int): bool = {
    val learned = known.get(receiver).union(known.get(sender))
    all {
      sender != receiver,
      not(known.get(sender).subseteq(known.get(receiver))),
      log' = log,
      known' = known.set(receiver, learned),
      views' = views.set(receiver, resolve(learned)),
    }
  }

  action step = {
    nondet author = DEVICES.oneOf()
    nondet target = DEVICES.oneOf()
    nondet sender = DEVICES.oneOf()
    nondet receiver = DEVICES.oneOf()
    val epoch = views.get(author).epoch + 1
    any {
      write(author, Add(target)),
      write(author, Remove(target)),
      write(author, Rotate(epoch)),
      pass(sender, receiver),
    }
  }

  // An account that a device knows keeps a member in its view.
  val kept = DEVICES.forall(d => known.get(d) == Set() or views.get(d).members != Set())
  // Devices that know the same operations draw the same members and epoch.
  val agreed = DEVICES.forall(a => DEVICES.forall(b => known.get(a) != known.get(b) or views.get(a) == views.get(b)))
  // Every member of a view was the founder or added by an operation the device knows.
  val vouched = DEVICES.forall(d => views.get(d).members.forall(member =>
    member == FOUNDER or known.get(d).exists(o => o.change == Add(member))))

  val safe = kept and agreed and vouched

  // The outcomes every simulation must reach.
  val joined = DEVICES.exists(d => views.get(d).members.size() > 1)
  val left = DEVICES.exists(d => known.get(d).exists(o => match o.change {
    | Add(device) => not(views.get(d).members.contains(device))
    | _ => false
  }))
  val rotated = DEVICES.exists(d => views.get(d).epoch > 0)
  pure def removes(o: Operation): bool = match o.change { Remove(_) => true | _ => false }
  val contested = log.exists(a => log.exists(b => a.id < b.id and a.parents == b.parents and removes(a) and removes(b)))
}

// The log as the membership rules implement it.
module membership {
  import membership_model(KEEP_LAST = true).*

  // A device joins, and two members that remove each other at once leave the one the replay reaches first.
  run contestTest = init
    .then(write(FOUNDER, Add(2)))
    .then(pass(FOUNDER, 2))
    .then(write(FOUNDER, Remove(2)))
    .then(write(2, Remove(FOUNDER)))
    .then(pass(2, FOUNDER))
    .then(all {
      assert(views.get(FOUNDER).members == Set(FOUNDER)),
      init,
    })

  // The last member cannot remove itself.
  run lastTest = init
    .then(write(FOUNDER, Remove(FOUNDER)))
    .then(all {
      assert(views.get(FOUNDER).members == Set(FOUNDER)),
      init,
    })
}

// The log without the guard on the last member, which must break `safe`.
module membership_probe {
  import membership_model(KEEP_LAST = false).*

  run orphanTest = init
    .then(write(FOUNDER, Remove(FOUNDER)))
    .then(all {
      assert(not(kept)),
      init,
    })
}
