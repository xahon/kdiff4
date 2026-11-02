use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};

use thiserror::Error;

use crate::messages::{Command, HostMessage, Request, RequestId};


/// Called by the host after each message it sends, so a sleeping front end wakes up and drains its client.
pub type Waker = Box<dyn Fn() + Send + Sync>;


type SharedWaker = Arc<Mutex<Option<Waker>>>;


#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransportError {
    #[error("the other endpoint is disconnected")]
    Disconnected,
}


/// Creates a connected client and host endpoint that pass messages over in-process channels.
pub fn in_process() -> (Client, HostEndpoint) {
    let (request_sender, request_receiver) = mpsc::channel();
    let (message_sender, message_receiver) = mpsc::channel();
    let waker: SharedWaker = Arc::new(Mutex::new(None));
    let client = Client { requests: request_sender, messages: message_receiver, waker: waker.clone(), next_id: 1 };
    let host = HostEndpoint { requests: request_receiver, messages: message_sender, waker };
    (client, host)
}


pub struct Client {
    requests: Sender<Request>,
    messages: Receiver<HostMessage>,
    waker: SharedWaker,
    next_id: u64,
}


impl Client {
    /// Replaces the waker and calls it once, so messages that arrived before it was set get drained.
    pub fn set_waker(&self, waker: Waker) {
        let mut slot = self.waker.lock().unwrap_or_else(PoisonError::into_inner);
        waker();
        *slot = Some(waker);
    }


    pub fn send(&mut self, command: Command) -> Result<RequestId, TransportError> {
        let id = RequestId(self.next_id.clone());
        self.requests.send(Request { id: id.clone(), command }).map_err(|_| TransportError::Disconnected)?;
        self.next_id += 1;
        Ok(id)
    }


    /// Never blocks: returns `Ok(None)` when no message is waiting.
    pub fn try_recv(&self) -> Result<Option<HostMessage>, TransportError> {
        match self.messages.try_recv() {
            Ok(message) => Ok(Some(message)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(TransportError::Disconnected),
        }
    }
}


pub struct HostEndpoint {
    requests: Receiver<Request>,
    messages: Sender<HostMessage>,
    waker: SharedWaker,
}


impl HostEndpoint {
    /// Blocks until a request arrives; returns `None` once the client is gone.
    pub fn recv(&self) -> Option<Request> {
        self.requests.recv().ok()
    }


    pub fn send(&self, message: HostMessage) -> Result<(), TransportError> {
        self.messages.send(message).map_err(|_| TransportError::Disconnected)?;
        let waker = self.waker.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(waker) = waker.as_ref() {
            waker();
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::messages::{Event, OpenFiles, Response, Revision, Snapshot};


    fn open_command() -> Command {
        Command::Open(OpenFiles { a: PathBuf::from("a.txt"), b: PathBuf::from("b.txt"), ..OpenFiles::default() })
    }


    fn ok_response(id: RequestId) -> HostMessage {
        HostMessage::Response(Response { id, result: Ok(()) })
    }


    #[test]
    fn requests_reach_host_with_increasing_ids() {
        let (mut client, host) = in_process();
        let first = client.send(open_command()).unwrap();
        let second = client.send(open_command()).unwrap();
        assert!(first < second);
        assert_eq!(host.recv().unwrap().id, first);
        let request = host.recv().unwrap();
        assert_eq!(request.id, second);
        assert_eq!(request.command, open_command());
    }


    #[test]
    fn client_receives_host_messages_in_order() {
        let (client, host) = in_process();
        assert_eq!(client.try_recv(), Ok(None));
        let snapshot = HostMessage::Event(Event::Snapshot(Snapshot { revision: Revision(1), documents: Vec::new(), output: None }));
        host.send(snapshot.clone()).unwrap();
        host.send(ok_response(RequestId(1))).unwrap();
        assert_eq!(client.try_recv(), Ok(Some(snapshot)));
        assert_eq!(client.try_recv(), Ok(Some(ok_response(RequestId(1)))));
        assert_eq!(client.try_recv(), Ok(None));
    }


    #[test]
    fn host_send_calls_waker() {
        let (client, host) = in_process();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        client.set_waker(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        host.send(ok_response(RequestId(1))).unwrap();
        host.send(ok_response(RequestId(2))).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }


    #[test]
    fn host_recv_ends_when_client_is_dropped() {
        let (client, host) = in_process();
        drop(client);
        assert!(host.recv().is_none());
        assert_eq!(host.send(ok_response(RequestId(1))), Err(TransportError::Disconnected));
    }


    #[test]
    fn client_reports_disconnected_host() {
        let (mut client, host) = in_process();
        drop(host);
        assert_eq!(client.try_recv(), Err(TransportError::Disconnected));
        assert_eq!(client.send(open_command()), Err(TransportError::Disconnected));
    }
}
