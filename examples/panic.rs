use {
    futures_timer::Delay,
    std::time::Duration,
    winit_executor::{block_on, spawn},
};

fn main() {
    block_on(async {
        let handle = spawn(async {
            seconds(5).await;
            panic!();
        });
        seconds(1).await;
        println!("delay1");
        handle.await;
        println!("delay2");
    });
}

fn seconds(seconds: u64) -> Delay {
    Delay::new(Duration::from_secs(seconds))
}
