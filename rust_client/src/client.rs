use crate::error::Error;
use humantime::format_duration;
use reqwest::{self, IntoUrl, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};

/// Returns an error for http status translating domain specific errors.
pub async fn error_for_status(response: Response) -> Result<Response, Error> {
    // Bad Request
    match response.status() {
        StatusCode::BAD_REQUEST => {
            // Since this is the client code which made the erroneous request, it is
            // supposedly caused by wrong arguments passed to this code by the
            // application. Let's forward the error as an exception, so the App
            // developer can act on it.
            let message = response.text().await?;
            // if message == "Unknown peer" {
            //     Error::UnknowPeer
            // }
            Err(Error::DomainError(message))
        }
        // This is returned by the server e.g. if requesting a lock with a count higher than max.
        StatusCode::CONFLICT => {
            let message = response.text().await?;
            Err(Error::DomainError(message))
        }
        _ => Ok(response.error_for_status()?),
    }
}

/// Send http requests to a throttle server. Only concerned with sending correct HTTP requests the
/// Throttle server understands. Not a higher level locking library.
#[derive(Debug, Clone)]
pub struct Client {
    client: reqwest::Client,
    url: Url,
}

impl Client {
    /// A new throttle Client instance.
    pub fn new(url: impl IntoUrl) -> Result<Self, Error> {
        let new_instance = Self {
            client: reqwest::Client::new(),
            url: url.into_url()?,
        };
        Ok(new_instance)
    }

    /// Register a new peer with the server.
    ///
    /// # Parameters
    ///
    /// * `expires_in`: Retention time of the peer on the server. A peer is used to acquire locks
    ///   and keep the leases to them alive. A Peer owns the locks which it acquires and releasing
    ///   it is going to release the owned locks as well.
    ///
    /// Every call to `new_peer` should be matched by a call to `release`.
    ///
    /// Creating a peer `new_peer` is separated from `acquire` in an extra Request mostly to make
    /// `acquire` idempotent. This prevents a call to acquire from acquiring more than one
    /// semaphore in case it is repeated due to a timeout.
    pub async fn new_peer(&self, expires_in: Duration) -> Result<u64, Error> {
        let response = self
            .client
            .post(self.url.join("new_peer").unwrap())
            .json(&ExpiresIn { expires_in })
            .send()
            .await?;
        let peer_id = error_for_status(response).await?.json().await?;
        Ok(peer_id)
    }

    /// Deletes the peer on the throttle server.
    ///
    /// This is important to unblock other clients which may be waiting for the semaphore remainder
    /// to increase.
    pub async fn release(&self, peer_id: u64) -> Result<(), Error> {
        let mut url = self.url.clone();
        url.path_segments_mut()
            .unwrap()
            .extend(["peers", &peer_id.to_string()]);
        let response = self.client.delete(url).send().await?;
        error_for_status(response).await?;
        Ok(())
    }

    /// Acquire a lock from the server.
    ///
    /// Every call to `acquire` should be matched by a call to `release`. Check out
    /// `lock` which as contextmanager does this for you.
    ///
    /// # Parameters
    ///
    /// * `semaphore`: Name of the semaphore to be acquired.
    /// * `count`: The count of the lock. A larger count represents a larger 'piece' of the resource
    ///   under procection.
    /// * `block_for`: The request returns as soon as the lock could be acquireod or after the
    ///   duration has elapsed, even if the lock could not be acquired. If set to `None`, the
    ///   request returns immediatly. Please note that this function is asynchronous and does not
    ///   block. The blocking does refer to the actual request. As such `block_for` represents an
    ///   upper bound after which the returned futures poll method is going to return `Ready`.
    /// * `expires_in`: The amount of time the remains valid. Can be prolonged by calling heartbeat.
    ///   After the time has passed the lock is considered released on the server side.
    ///
    /// # Return
    ///
    /// `true` if the lock is active, `false` otherwise.
    pub async fn acquire(
        &self,
        peer_id: u64,
        semaphore: &str,
        count: u32,
        expires_in: Option<Duration>,
        block_for: Option<Duration>,
    ) -> Result<bool, Error> {
        let mut url = self.url.clone();
        url.path_segments_mut()
            .unwrap()
            .extend(["peers", &peer_id.to_string(), semaphore]);

        {
            let mut query = url.query_pairs_mut();
            query.extend_pairs(expires_in.map(|d| ("expires_in", format_duration(d).to_string())));
            query.extend_pairs(block_for.map(|d| ("block_for", format_duration(d).to_string())));
        }
        let response = self.client.put(url).json(&count).send().await?;
        let response = error_for_status(response).await?;
        match response.status() {
            StatusCode::OK => Ok(true),
            StatusCode::ACCEPTED => Ok(false),
            _ => Err(Error::UnexpectedResponse),
        }
    }

    /// Ask the server wether the peer has acquired all its locks.
    pub async fn is_acquired(&self, peer_id: u64) -> Result<bool, Error> {
        let mut url = self.url.clone();
        url.path_segments_mut()
            .unwrap()
            .extend(["peers", &peer_id.to_string(), "is_acquired"]);
        let response = self.client.get(url).send().await?;
        let acquired = error_for_status(response).await?.json().await?;
        Ok(acquired)
    }

    /// The curent semaphore count. I.e. the number of available leases
    ///
    /// This is equal to the full semaphore count minus the current count. This number
    /// could become negative, if the semaphores have been overcommitted (due to
    /// previously reoccuring leases previously considered dead).
    pub async fn remainder(&self, semaphore: &str) -> Result<i64, Error> {
        let mut url = self.url.join("remainder").unwrap();
        url.query_pairs_mut().append_pair("semaphore", semaphore);

        let response = self.client.get(url).send().await?;
        let remainder = error_for_status(response).await?.json().await?;

        Ok(remainder)
    }

    /// Sends a restore request to the server.
    ///
    /// This request creates a peer with the specified peer id. The peer is also going to hold all
    /// the locks passed in acquired, even if this means exceeding the semaphore count.
    pub async fn restore(
        &self,
        peer_id: u64,
        acquired: &HashMap<String, u32>,
        expires_in: Duration,
    ) -> Result<(), Error> {
        let url = self.url.join("restore").unwrap();
        let response = self
            .client
            .post(url)
            .json(&Restore {
                expires_in,
                peer_id,
                acquired,
            })
            .send()
            .await?;
        error_for_status(response).await?;
        Ok(())
    }

    /// Send a PUT request to the server updating the expiration timestamp
    pub async fn heartbeat(&self, peer_id: u64, expires_in: Duration) -> Result<(), Error> {
        let mut url = self.url.clone();
        url.path_segments_mut()
            .unwrap()
            .extend(["peers", &peer_id.to_string()]);
        let response = self
            .client
            .put(url)
            .json(&ExpiresIn { expires_in })
            .send()
            .await?;
        error_for_status(response).await?;
        Ok(())
    }

    /// Release a lock to a semaphore for a specific peer
    pub async fn release_lock(&self, peer_id: u64, semaphore: &str) -> Result<(), Error> {
        let mut url = self.url.clone();
        url.path_segments_mut()
            .unwrap()
            .extend(["peers", &peer_id.to_string(), semaphore]);
        let response = self.client.delete(url).send().await?;
        error_for_status(response).await?;
        Ok(())
    }

    pub async fn list_of_peers(&self) -> Result<Vec<PeerDescription>, Error> {
        let mut url = self.url.clone();
        url.path_segments_mut()
            .unwrap()
            // Use empty string to achieve trailing slash (`/`) in path
            .extend(["peers", ""]);
        let response = self.client.get(url).send().await?;
        let peers = error_for_status(response).await?.json().await?;
        Ok(peers)
    }
}

/// The properties and state of a peer, as returned by throttle then listing peers.
#[derive(Deserialize)]
pub struct PeerDescription {}

/// Used as a query parameter in requests. E.g. `?expires_in=5m`.
#[derive(Serialize)]
struct ExpiresIn {
    #[serde(with = "humantime_serde")]
    expires_in: Duration,
}

#[derive(Serialize)]
struct Restore<'a> {
    #[serde(with = "humantime_serde")]
    expires_in: Duration,
    peer_id: u64,
    acquired: &'a HashMap<String, u32>,
}
