use {
    crossbeam::{channel, sync::Parker},
    futures::FutureExt,
    once_cell::sync::Lazy,
    std::{
        cell::RefCell,
        future::Future,
        panic::{resume_unwind, AssertUnwindSafe},
        pin::Pin,
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

pub struct JoinHandle<R>(async_task::JoinHandle<thread::Result<R>, ()>);
impl<R> Future for JoinHandle<R> {
    type Output = Option<R>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.0).poll(cx) {
            Poll::Pending => Poll::Pending,
            // Cancelled
            Poll::Ready(None) => Poll::Ready(None),
            // Succeeded
            Poll::Ready(Some(Ok(val))) => Poll::Ready(Some(val)),
            // Caught panic
            Poll::Ready(Some(Err(err))) => resume_unwind(err),
        }
    }
}

pub fn spawn<F, R>(future: F) -> JoinHandle<R>
where
    F: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    RUNTIME.spawn(future)
}

type Task = async_task::Task<()>;

static RUNTIME: Lazy<Runtime> = Lazy::new(|| {
    let runtime = Runtime::new();
    runtime.start_threaded();
    runtime
});

pub struct Runtime {
    queue: channel::Sender<Task>,
    stream: channel::Receiver<Task>,
}
impl Runtime {
    pub fn new() -> Self {
        let (sender, receiver) = channel::unbounded::<Task>();

        Runtime {
            queue: sender,
            stream: receiver,
        }
    }
    pub fn start_threaded(&self) {
        for _ in 0..num_cpus::get().max(1) {
            let receiver = self.stream.clone();
            thread::spawn(move || {
                for task in receiver.iter() {
                    task.run();
                }
            });
        }
    }
    pub fn spawn<F, R>(&self, future: F) -> JoinHandle<R>
    where
        F: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let future = AssertUnwindSafe(future).catch_unwind();

        let queue = self.queue.clone();
        let (task, handle) = async_task::spawn(future, move |t| queue.send(t).unwrap(), ());

        task.schedule();

        JoinHandle(handle)
    }
}
