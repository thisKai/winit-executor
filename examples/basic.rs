use {executor::block_on, futures_timer::Delay, std::time::Duration};

pub fn main() {
    block_on(async {
        Delay::new(Duration::from_secs(1)).await;
        println!("delay");
    });
}
