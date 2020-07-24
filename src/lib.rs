use std::{future::Future, task::{Context, Poll}, thread};

pub fn block_on<F: Future>(future: F) -> F::Output {
    pin_utils::pin_mut!(future);

    let thread = thread::current();
    let waker = async_task::waker_fn(move || thread.unpark());

    let cx = &mut Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}
