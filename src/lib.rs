use {
    crossbeam::{channel, sync::Parker},
    once_cell::sync::Lazy,
    std::{
        cell::RefCell,
        future::Future,
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

type JoinHandle<R> = Pin<Box<dyn Future<Output = R> + Send>>;

pub fn spawn<F, R>(future: F) -> JoinHandle<R>
where
    F: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let (task, handle) = async_task::spawn(future, |t| QUEUE.send(t).unwrap(), ());

    task.schedule();

    Box::pin(async { handle.await.unwrap() })
}

type Task = async_task::Task<()>;

static QUEUE: Lazy<channel::Sender<Task>> = Lazy::new(|| {
    let (sender, receiver) = channel::unbounded::<Task>();

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
