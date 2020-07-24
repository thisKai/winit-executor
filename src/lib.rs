use {
    crossbeam::sync::Parker,
    std::{
        future::Future,
        task::{Context, Poll},
    },
};

pub fn block_on<F: Future>(future: F) -> F::Output {
    pin_utils::pin_mut!(future);

    let parker = Parker::new();
    let unparker = parker.unparker().clone();
    let waker = async_task::waker_fn(move || unparker.unpark());

    let cx = &mut Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => parker.park(),
        }
    }
}
