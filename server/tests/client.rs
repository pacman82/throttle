mod common;

use common::Server;

use std::{
    collections::HashMap,
    time::Duration,
};

use tokio::time::timeout;


/// `client.acquire` if called in a non-blocking fashion, must return `true` if the lock can be
/// acquired immediatly and `false` otherwise. This must also be in affirmed by subsequent calls
/// to `client.is_acquired`.
#[tokio::test]
async fn non_blocking_acquire() {
    let config = "[semaphores]\nA=1\n";
    let server = Server::new(8001, config);
    let client = server.make_client();

    // Allocate to peers
    let first = client.new_peer(Duration::from_secs(1)).await.unwrap();
    let second = client.new_peer(Duration::from_secs(1)).await.unwrap();

    // Acquire a lock with the first peer.
    assert!(client.acquire(first, "A", 1, None, None).await.unwrap());
    assert!(client.is_acquired(first).await.unwrap());

    // Second lease is pending, because we still hold first
    assert!(!client.acquire(second, "A", 1, None, None).await.unwrap());
    assert!(!client.is_acquired(second).await.unwrap());

    // Release the first peer, this implicitly will also free the lock associated with it.
    client.release(first).await.unwrap();

    // The second peer should have acquired its lock. Note that we did not repeat the call to
    // `acquire`.
    assert!(client.is_acquired(second).await.unwrap());
}

/// If a peer is removed due to expiration, it's locks must be released.
#[tokio::test]
async fn locks_of_expired_peers_are_released() {
    let config = "[semaphores]\nA=1\n";
    let server = Server::new(8002, config);
    let client = server.make_client();

    // Create a peer which expires immediately
    let peer = client.new_peer(Duration::from_secs(0)).await.unwrap();
    client.acquire(peer, "A", 1, None, None).await.unwrap();
    assert_eq!(1, client.remove_expired().await.unwrap());
    // Semaphore should again be available
    assert_eq!(1, client.remainder("A").await.unwrap());
}

/// Future returned by `client.acquire` must not be ready until the semaphore count allows for
/// the lock to be acquired.
#[tokio::test]
async fn acquiring_with_blocks_for() {
    let config = "[semaphores]\nA=1\n";
    let server = Server::new(8003, config);
    let client = server.make_client();

    let first = client.new_peer(Duration::from_secs(5)).await.unwrap();
    let second = client.new_peer(Duration::from_secs(5)).await.unwrap();

    // Acquire lock to A with `first`. This is going to block `second` from doing the same
    // thing.
    client.acquire(first, "A", 1, None, None).await.unwrap();

    let block_for = Some(Duration::from_secs(1));
    let future = client.acquire(second, "A", 1, None, block_for);
    let timeout = tokio::time::timeout(Duration::from_millis(100), future).await;
    assert!(timeout.is_err());
}

/// Acquire must be able to prolong the lifetime of peers, in order for peers not to expire while
/// repeating calls to acquire during waiting for a lock.
#[tokio::test]
async fn acquire_prolongs_lifetime_of_peer() {
    let config = "[semaphores]\nA=1\n";
    let server = Server::new(8004, config);
    let client = server.make_client();

    let blocker = client.new_peer(Duration::from_secs(10)).await.unwrap();

    // Acquire `A` so subsequent locks are pending
    client.acquire(blocker, "A", 1, None, None).await.unwrap();

    let peer = client.new_peer(Duration::from_millis(100)).await.unwrap();

    // This lock can not be acquired due to `blocker` holding the lock. This request is going to
    // block for one second. After which the peer should have been expired. Yet acquire can
    // prolong the lifetime of the peer.
    client
        .acquire(
            peer,
            "A",
            1,
            Some(Duration::from_secs(10)),
            Some(Duration::from_millis(100)),
        )
        .await
        .unwrap();

    // The initial timeout of 100ms should have been expired by now, yet nothing is removed.
    assert_eq!(0, client.remove_expired().await.unwrap());
}

/// A revenant Peer may not acquire a semaphore which does not exist on the server.
#[tokio::test]
async fn restore_peer_with_unknown_semaphore() {
    // Setup a server without any semaphores.
    let config = "[semaphores]";
    let server = Server::new(8005, config);
    let client = server.make_client();

    // Bogus peer id, presumably from a previous run, before lock losts its state
    let peer = 5;
    let mut acquired = HashMap::new();
    acquired.insert(String::from("A"), 1);
    let error = client
        .restore(peer, &acquired, Duration::from_secs(1))
        .await
        .err()
        .unwrap();

    assert_eq!(
        error.to_string(),
        "Throttle client domain error: Unknown semaphore"
    );
}

/// A large lock mustnot get starved by many smaller ones.
#[tokio::test]
async fn do_not_starve_large_locks() {
    let config = "[semaphores]\nA=5";
    let server = Server::new(8006, config);
    let client = server.make_client();

    let small = client.new_peer(Duration::from_secs(1)).await.unwrap();
    let big = client.new_peer(Duration::from_secs(1)).await.unwrap();
    let other_small = client.new_peer(Duration::from_secs(1)).await.unwrap();
    // This lock is acquired immediatly decrementing the semaphore count to 4
    assert!(client.acquire(small, "A", 1, None, None).await.unwrap());
    // Now try a large one. Of course we can not acquire it yet
    assert!(!client.acquire(big, "A", 5, None, None).await.unwrap());
    // This one could be acquired due to semaphore count, but won't, since the larger one is
    // still pending.
    assert!(!client
        .acquire(other_small, "A", 1, None, None)
        .await
        .unwrap());
    // Remainder is still 4
    assert_eq!(4, client.remainder("A").await.unwrap());

    // We free the first small lock, now the big one is acquired
    client.release(small).await.unwrap();
    assert_eq!(0, client.remainder("A").await.unwrap());
}

/// Given a semaphore count of three, three leases can be acquired simultaniously.
#[tokio::test]
async fn acquire_three_leases() {
    let config = "[semaphores]\nA=3";
    let server = Server::new(8007, config);
    let client = server.make_client();

    let mut p = Vec::new();
    for _ in 0..4 {
        p.push(client.new_peer(Duration::from_secs(1)).await.unwrap());
    }

    assert!(client.acquire(p[0], "A", 1, None, None).await.unwrap());
    assert!(client.acquire(p[1], "A", 1, None, None).await.unwrap());
    assert!(client.acquire(p[2], "A", 1, None, None).await.unwrap());
    assert_eq!(0, client.remainder("A").await.unwrap());
    assert!(!client.acquire(p[3], "A", 1, None, None).await.unwrap());
}

/// `acquire` must return immediatly after the pending lock can be acquired and not wait for the
/// next request to go through.
#[tokio::test]
async fn unblock_immediatly_after_release() {
    let config = "[semaphores]\nA=1";
    let server = Server::new(8008, config);
    let client = server.make_client();

    let one = client.new_peer(Duration::from_secs(1)).await.unwrap();
    let two = client.new_peer(Duration::from_secs(1)).await.unwrap();

    // Acquire first lease
    client.acquire(one, "A", 1, None, None).await.unwrap();

    // Don't block on this right away, since it can't be completed while `one` has the first
    // lease.
    let client_2 = client.clone();
    let wait_for_two = tokio::spawn(async move {
        client_2
            .acquire(two, "A", 1, None, Some(Duration::from_secs(15)))
            .await
    });

    let release_one = client.release(one);

    // Two is a allowed to block for 15 seconds, yet it must not time out in even one second
    // since we release one.
    let (wait_for_two, _) =
        tokio::join!(timeout(Duration::from_secs(1), wait_for_two), release_one);
    assert!(wait_for_two.unwrap().unwrap().unwrap());
}

/// Server should answer request to `acquire` if the timeout has elapsed.
#[tokio::test]
async fn server_side_timeout() {
    let config = "[semaphores]\nA=1";
    let server = Server::new(8009, config);
    let client = server.make_client();

    let one = client.new_peer(Duration::from_secs(1)).await.unwrap();
    let two = client.new_peer(Duration::from_secs(1)).await.unwrap();

    // Acquire first lease
    client.acquire(one, "A", 1, None, None).await.unwrap();

    // Wait for two in a seperate thread so we do not block forever if this test fails.
    assert!(!timeout(
        Duration::from_secs(1),
        client.acquire(two, "A", 1, None, Some(Duration::from_millis(1))),
    )
    .await
    .unwrap() // <-- Did not timeout
    .unwrap());
}

// `acquire` must return immediatly after the pending lock can be acquired and not wait for the
// next request to go through. In this test the peer previously holding the lock expires rather
// than being deleted explicitly.
#[tokio::test]
async fn acquire_locks_immediatly_after_expiration() {
    let config = "litter_collection_interval = \"10ms\"\n[semaphores]\nA=1";
    let server = Server::new(8010, config);
    let client = server.make_client();

    let one = client.new_peer(Duration::from_secs(1)).await.unwrap();
    let two = client.new_peer(Duration::from_secs(1)).await.unwrap();

    // Acquire first lease
    client.acquire(one, "A", 1, None, None).await.unwrap();
    let wait_for_two = client.acquire(two, "A", 1, None, Some(Duration::from_millis(500)));
    // Sending this heartbeat is marking `one` as expired immediatly. `one` is going to be
    // picked up by the litter collection and thus the lock it holds being released, unblocking
    // `two`.
    let heartbeat = client.heartbeat(one, Duration::from_secs(0));

    let (wait_for_two, _) = tokio::join!(wait_for_two, heartbeat);
    assert!(wait_for_two.unwrap());
}

/// `acquire` must return `False` while pending and `True` once lock is acquired.
#[tokio::test]
async fn acquire() {
    let config = "[semaphores]\nA=1";
    let server = Server::new(8011, config);
    let client = server.make_client();

    let one = client.new_peer(Duration::from_secs(1)).await.unwrap();
    let two = client.new_peer(Duration::from_secs(1)).await.unwrap();

    // Acquire first lease
    assert!(client.acquire(one, "A", 1, None, None).await.unwrap());
    // Second must be pending
    assert!(!client.acquire(two, "A", 1, None, None).await.unwrap());
    // Release one, so second is acquired
    client.release(one).await.unwrap();
    assert!(client.acquire(two, "A", 1, None, None).await.unwrap());
}

/// Releasing a lock, while keeping its peer, must still enable other locks to be acquired.
#[tokio::test]
async fn acquire_two_locks_with_one_peer() {
    let config = "[semaphores.A]
        max = 1
        level = 1
        [semaphores.B]
        max = 1
        level = 0
    ";
    let server = Server::new(8012, config);
    let client = server.make_client();

    let one = client.new_peer(Duration::from_secs(1)).await.unwrap();
    let two = client.new_peer(Duration::from_secs(2)).await.unwrap();

    // Acquire locks to two semaphores with same peer
    assert!(client.acquire(one, "A", 1, None, None).await.unwrap());
    assert!(client.acquire(one, "B", 1, None, None).await.unwrap());

    assert!(!client.acquire(two, "B", 1, None, None).await.unwrap());

    // Release one "B", so two is acquired
    client.release_lock(one, "B").await.unwrap();

    assert!(client.is_acquired(two).await.unwrap());
    // First peer is still active and holds a lock
    assert!(client.is_acquired(one).await.unwrap());
}

/// Locks of unresponsive clients must be freed by litter collection.
#[tokio::test]
async fn litter_collection() {
    let config = r#" litter_collection_interval="10ms"
        [semaphores]
        A = 1
        "#;
    let server = Server::new(8013, config);
    let client = server.make_client();

    // Acquire lock but never release it.
    let peer = client.new_peer(Duration::from_secs(1)).await.unwrap();
    client
        .acquire(peer, "A", 1, Some(Duration::from_millis(1)), None)
        .await
        .unwrap();
    std::thread::sleep(Duration::from_millis(15));
    assert_eq!(1, client.remainder("A").await.unwrap());
}
