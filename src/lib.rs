use {
    crossbeam::{channel, sync::Parker},
    futures::channel::oneshot,
    once_cell::sync::Lazy,
    std::{
        cell::RefCell,
        future::Future,
        pin::Pin,
        sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex},
        task::{Context, Poll, Waker},
        thread,
    },
};

pub fn block_on<F: Future>(future: F) -> F::Output {
    futures::pin_mut!(future);

    thread_local! {
        static CACHE: RefCell<(Parker, Waker)> = {
            let parker = Parker::new();
            let unparker = parker.unparker().clone();
            let waker = async_task::waker_fn(move || unparker.unpark());
            RefCell::new((parker, waker))
        };
    }

    CACHE.with(|cache| {
        let (parker, waker) = &mut *cache.try_borrow_mut().expect("recursive `block_on`");
        let cx = &mut Context::from_waker(&waker);
        loop {
            match future.as_mut().poll(cx) {
                Poll::Ready(output) => return output,
                Poll::Pending => parker.park(),
            }
        }
    })
}

type JoinHandle<R> = Pin<Box<dyn Future<Output = R> + Send>>;

pub fn spawn<F, R>(future: F) -> JoinHandle<R>
where
    F: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let (s, r) = oneshot::channel();
    let future = async move {
        let _ = s.send(future.await);
    };

    let task = Arc::new(Task {
        state: AtomicUsize::new(0),
        future: Mutex::new(Box::pin(future)),
    });

    QUEUE.send(task).unwrap();

    Box::pin(async { r.await.unwrap() })
}


const WOKEN: usize = 0b01;
const RUNNING: usize = 0b10;

struct Task {
    state: AtomicUsize,
    future: Mutex<Pin<Box<dyn Future<Output = ()> + Send>>>,
}
impl Task {
    pub fn run(self: Arc<Self>) {
        let task = self.clone();
        let waker = async_task::waker_fn(move || {
            // schedule if the task is not woken already and is not running
            if task.state.fetch_or(WOKEN, Ordering::SeqCst) == 0 {
                QUEUE.send(task.clone()).unwrap();
            }
        });

        self.state.store(RUNNING, Ordering::SeqCst);
        let cx = &mut Context::from_waker(&waker);
        let poll = self.future.try_lock().unwrap().as_mut().poll(cx);

        // schedule if the task was woken while running
        if poll.is_pending() {
            if self.state.fetch_and(!RUNNING, Ordering::SeqCst) == WOKEN | RUNNING {
                QUEUE.send(self).unwrap();
            }
        }
    }
}

static QUEUE: Lazy<channel::Sender<Arc<Task>>> = Lazy::new(|| {
    let (sender, receiver) = channel::unbounded::<Arc<Task>>();

    for _ in 0..num_cpus::get().max(1) {
        let receiver = receiver.clone();
        thread::spawn(move || {
            for task in receiver.iter() {
                task.run();
            }
        });
    }

    sender
});
