use std::sync::{Mutex, RwLock};

use futures::channel::oneshot;

/// A message queue with persistent history that can be subscribed to.
///
/// Definitely non-optimal. This would ideally be a lockless mpmc queue.
#[derive(Default)]
pub struct MessageQueue<T> {
    messages: RwLock<Vec<T>>,
    message_listeners: Mutex<Vec<Listener<T>>>,
    #[cfg(test)]
    before_listener_lock: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl<T: Clone> MessageQueue<T> {
    pub fn new() -> MessageQueue<T> {
        MessageQueue {
            messages: RwLock::new(Vec::new()),
            message_listeners: Mutex::new(Vec::new()),
            #[cfg(test)]
            before_listener_lock: Mutex::new(None),
        }
    }

    pub fn push_messages(&self, new_messages: &[T]) {
        let mut message_listeners = self.message_listeners.lock().unwrap();
        let mut messages = self.messages.write().unwrap();
        messages.extend_from_slice(new_messages);

        let mut remaining_listeners = Vec::new();

        for listener in message_listeners.drain(..) {
            match fire_listener_if_ready(&messages, listener) {
                Ok(_) => {}
                Err(listener) => remaining_listeners.push(listener),
            }
        }

        // Without this annotation, Rust gets confused since the first argument
        // is a MutexGuard, but the second is a Vec.
        *message_listeners = remaining_listeners;
    }

    /// Subscribe to any messages occurring after the given message cursor.
    pub fn subscribe(&self, cursor: u32) -> oneshot::Receiver<(u32, Vec<T>)> {
        let (sender, receiver) = oneshot::channel();

        #[cfg(test)]
        if let Some(hook) = self.before_listener_lock.lock().unwrap().take() {
            hook();
        }
        // Match push_messages' lock order. Otherwise a push can occur between
        // the empty-history check and registration and never wake this listener.
        let mut message_listeners = self.message_listeners.lock().unwrap();

        let listener = {
            let listener = Listener { sender, cursor };

            let messages = self.messages.read().unwrap();

            match fire_listener_if_ready(&messages, listener) {
                Ok(_) => return receiver,
                Err(listener) => listener,
            }
        };

        message_listeners.push(listener);

        receiver
    }

    /// Subscribe to any messages being pushed into the queue.
    ///
    /// This method is only useful in tests. Non-test code should use subscribe
    /// instead.
    #[cfg(test)]
    #[allow(unused)]
    pub fn subscribe_any(&self) -> oneshot::Receiver<(u32, Vec<T>)> {
        let cursor = {
            let messages = self.messages.read().unwrap();
            messages.len() as u32
        };

        self.subscribe(cursor)
    }

    pub fn cursor(&self) -> u32 {
        self.messages.read().unwrap().len() as u32
    }
}

#[cfg(test)]
mod test {
    use super::MessageQueue;
    use std::{
        sync::{mpsc, Arc},
        thread,
        time::Duration,
    };

    #[test]
    fn subscribe_cannot_miss_a_message_before_listener_registration() {
        let queue = Arc::new(MessageQueue::new());
        let (before_lock, reached_lock) = mpsc::channel();
        let (release_lock, proceed) = mpsc::channel();
        *queue.before_listener_lock.lock().unwrap() = Some(Box::new(move || {
            before_lock.send(()).unwrap();
            proceed.recv_timeout(Duration::from_secs(2)).unwrap();
        }));
        let subscriber = Arc::clone(&queue);
        let thread = thread::spawn(move || subscriber.subscribe(0));
        reached_lock.recv_timeout(Duration::from_secs(2)).unwrap();
        queue.push_messages(&[7]);
        release_lock.send(()).unwrap();
        let mut receiver = thread.join().unwrap();
        assert_eq!(receiver.try_recv().unwrap(), Some((1, vec![7])));
    }
}

struct Listener<T> {
    sender: oneshot::Sender<(u32, Vec<T>)>,
    cursor: u32,
}

fn fire_listener_if_ready<T: Clone>(
    messages: &[T],
    listener: Listener<T>,
) -> Result<(), Listener<T>> {
    let current_cursor = messages.len() as u32;

    if listener.cursor < current_cursor {
        let new_messages = messages[(listener.cursor as usize)..].to_vec();
        let _ = listener.sender.send((current_cursor, new_messages));
        Ok(())
    } else {
        Err(listener)
    }
}
