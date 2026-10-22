// The screen machine of device_rules::Run, with elapsed times around its five-minute deadline.
// device_replay.rs checks these traces against production Rust, including cancellation after membership is granted.
module device_model {
  const CHECK_DEADLINE: bool
  pure val PATIENCE = 300000
  pure val EVENTS = Set("Poll", "Ready", "Peer", "Joined", "Synced", "Refuse", "Cancel")
  pure val TIMES = Set(0, 1, PATIENCE - 1, PATIENCE, PATIENCE + 1)

  pure def terminal(s: str): bool = Set("Done", "Expired", "Refused", "Cancelled").contains(s)
  pure def rank(s: str): int =
    if (s == "Offered") 0 else if (s == "Waiting") 1
    else if (s == "Confirming") 2 else if (s == "Syncing") 3 else 4

  pure def advance(s: str, e: str, t: int): str =
    if (terminal(s)) s
    else if (s == "Syncing") {
      if (e == "Synced") "Done" else if (e == "Refuse") "Refused" else "Syncing"
    }
    else if (e == "Cancel") "Cancelled"
    else if (CHECK_DEADLINE and t >= PATIENCE) "Expired"
    else if (e == "Refuse") "Refused"
    else if (s == "Offered" and e == "Ready") "Waiting"
    else if (s == "Waiting" and e == "Peer") "Confirming"
    else if (s == "Confirming" and e == "Joined") "Syncing"
    else s

  var joining: bool
  var stage: str
  var event: str
  var now: int
  var kept: bool

  action init = {
    nondet join = Set(false, true).oneOf()
    all {
      joining' = join,
      stage' = if (join) "Waiting" else "Offered",
      event' = "Poll",
      now' = 0,
      kept' = true,
    }
  }

  action apply(e: str, t: int): bool = {
    val after = advance(stage, e, t)
    all {
      joining' = joining,
      stage' = after,
      event' = e,
      now' = t,
      kept' = (rank(after) >= rank(stage)
        and (not(terminal(stage)) or after == stage)
        and (terminal(stage) or stage == "Syncing" or t < PATIENCE
          or Set("Expired", "Cancelled").contains(after))),
    }
  }

  action step = {
    nondet e = EVENTS.oneOf()
    nondet t = TIMES.oneOf()
    apply(e, t)
  }

  val safe = kept
  val completed = stage == "Done"
  val expired = stage == "Expired"
  val refused = stage == "Refused"
  val cancelled = stage == "Cancelled"
  val granted = stage == "Syncing"
}

module devices {
  import device_model(CHECK_DEADLINE = true).*

  run expiredTest = init.then(apply("Joined", PATIENCE)).then(all {
    assert(stage == "Expired"), init,
  })

  run grantedTest = init.then(apply("Ready", 0)).then(apply("Peer", 0))
    .then(apply("Joined", 0)).then(apply("Cancel", PATIENCE))
    .then(apply("Synced", PATIENCE)).then(all {
      assert(stage == "Done" and safe), init,
    })
}

module devices_probe {
  import device_model(CHECK_DEADLINE = false).*

  run ignoredDeadlineTest = init.then(apply("Ready", PATIENCE)).then(all {
    assert(not(safe)), init,
  })
}
