use {
    futures_timer::Delay,
    std::time::Duration,
    winit::{
        event::{Event, WindowEvent},
        event_loop::ControlFlow,
        window::WindowBuilder,
    },
    winit_executor::EventLoop,
};

pub fn main() {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("window")
        .build(&event_loop)
        .unwrap();
    event_loop.run(|event, target, control_flow| match event {
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } => {
            *control_flow = ControlFlow::Exit;
        }
        Event::WindowEvent {
            event: WindowEvent::CursorEntered { .. },
            ..
        } => {
            target.spawn(async {
                seconds(1).await;
                println!("delay1");
            });
        }
        _ => {}
    });
}

fn seconds(seconds: u64) -> Delay {
    Delay::new(Duration::from_secs(seconds))
}
