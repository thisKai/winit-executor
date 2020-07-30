use {
    crossbeam::{channel, sync::Parker},
    futures::FutureExt,
    once_cell::sync::Lazy,
    std::{
        cell::RefCell,
        future::Future,
        ops::Deref,
        panic::{resume_unwind, AssertUnwindSafe},
        pin::Pin,
        task::{Context, Poll, Waker},
        thread,
    },
};

pub struct EventLoop<T: 'static> {
    event_loop: winit::event_loop::EventLoop<T>,
    runtime: Runtime,
}
impl EventLoop<()> {
    pub fn new() -> Self {
        Self::with_user_event()
    }
}
impl<T> EventLoop<T> {
    pub fn with_user_event() -> Self {
        let event_loop = winit::event_loop::EventLoop::with_user_event();
        Self {
            event_loop,
            runtime: Runtime::new(),
        }
    }
    pub fn spawn<F, R>(&self, future: F) -> JoinHandle<R>
    where
        F: Future<Output = R> + 'static,
        R: 'static,
    {
        self.spawner().spawn(future)
    }
    pub fn spawner(&self) -> &Spawner {
        &self.runtime.spawner
    }
    pub fn run<F>(self, mut event_handler: F) -> !
    where
        F: 'static
            + FnMut(
                winit::event::Event<T>,
                &EventLoopWindowTarget<T>,
                &mut winit::event_loop::ControlFlow,
            ),
    {
        use winit::event::Event;
        let Self {
            event_loop,
            runtime,
        } = self;
        event_loop.run({
            move |event, target, control_flow| {
                if let Event::RedrawEventsCleared = event {
                    for task in runtime.poll_task() {
                        task.run();
                    }
                }
                event_handler(event, &runtime.target(target), control_flow)
            }
        })
    }
    pub fn create_proxy(&self) -> winit::event_loop::EventLoopProxy<T> {
        self.event_loop.create_proxy()
    }
}
impl<T> Deref for EventLoop<T> {
    type Target = winit::event_loop::EventLoopWindowTarget<T>;
    fn deref(&self) -> &Self::Target {
        &*self.event_loop
    }
}

#[derive(Clone)]
pub struct Spawner {
    queue: channel::Sender<Task>,
}
impl Spawner {
    fn queue(&self, task: Task) {
        self.queue.send(task).unwrap();
    }
    pub fn spawn<F, R>(&self, future: F) -> JoinHandle<R>
    where
        F: Future<Output = R> + 'static,
        R: 'static,
    {
        let future = AssertUnwindSafe(future).catch_unwind();

        let spawner = self.clone();
        let (task, handle) = async_task::spawn_local(future, move |t| spawner.queue(t), ());

        task.run();

        JoinHandle(handle)
    }
}
pub struct EventLoopWindowTarget<'a, T: 'static> {
    target: &'a winit::event_loop::EventLoopWindowTarget<T>,
    spawner: Spawner,
}
impl<T> EventLoopWindowTarget<'_, T> {
    pub fn spawn<F, R>(&self, future: F) -> JoinHandle<R>
    where
        F: Future<Output = R> + 'static,
        R: 'static,
    {
        self.spawner.spawn(future)
    }
    pub fn spawner(&self) -> &Spawner {
        &self.spawner
    }
}
impl<T> Deref for EventLoopWindowTarget<'_, T> {
    type Target = winit::event_loop::EventLoopWindowTarget<T>;
    fn deref(&self) -> &Self::Target {
        &self.target
    }
}

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
    spawner: Spawner,
    stream: channel::Receiver<Task>,
}
impl Runtime {
    pub fn new() -> Self {
        let (sender, receiver) = channel::unbounded::<Task>();

        Runtime {
            spawner: Spawner { queue: sender },
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
        self.spawner.spawn(future)
    }
    pub fn spawner(&self) -> Spawner {
        self.spawner.clone()
    }
    fn target<'a, T>(
        &self,
        target: &'a winit::event_loop::EventLoopWindowTarget<T>,
    ) -> EventLoopWindowTarget<'a, T> {
        EventLoopWindowTarget {
            target,
            spawner: self.spawner.clone(),
        }
    }
    fn iter(&self) -> impl Iterator<Item = Task> + '_ {
        self.stream.iter()
    }
    fn poll_task(&self) -> Option<Task> {
        self.stream.try_recv().ok()
    }
}
