use std::{sync::Arc, time::Duration};

use breakwater_core::{framebuffer::FrameBuffer, test::helpers::DevNullTcpStream};
use breakwater_parser::{
    implementations::{AssemblerParser, SimpleParser},
    Parser,
};
use criterion::{criterion_group, criterion_main, Criterion};
use pixelbomber::image_handler::{self, ImageConfigBuilder};

const FRAMEBUFFER_WIDTH: usize = 1920;
const FRAMEBUFFER_HEIGHT: usize = 1080;

fn compare_implementations(c: &mut Criterion) {
    invoke_benchmark(
        c,
        "parse_draw_commands_unordered",
        "benches/non-transparent.png",
        true,
        false,
        false,
    );
    invoke_benchmark(
        c,
        "parse_draw_commands_ordered",
        "benches/non-transparent.png",
        false,
        false,
        false,
    );
    invoke_benchmark(
        c,
        "parse_draw_commands_with_offset",
        "benches/non-transparent.png",
        true,
        true,
        false,
    );
    invoke_benchmark(
        c,
        "parse_mixed_draw_commands",
        "benches/mixed.png",
        false,
        false,
        true,
    );
}

fn invoke_benchmark(
    c: &mut Criterion,
    bench_name: &str,
    image: &str,
    shuffle: bool,
    use_offset: bool,
    use_gray: bool,
) {
    let commands = image_handler::load(
        vec![image],
        &ImageConfigBuilder::new()
            .width(FRAMEBUFFER_WIDTH as u32)
            .height(FRAMEBUFFER_HEIGHT as u32)
            .shuffle(shuffle)
            .offset_usage(use_offset)
            .gray_usage(use_gray)
            .build(),
    )
    .pop()
    .expect("Fail to retrieve Pixelflut commands");

    let mut c_group = c.benchmark_group(bench_name);

    c_group.bench_with_input("Simple", &commands, |b, input| {
        let fb = Arc::new(FrameBuffer::new(FRAMEBUFFER_WIDTH, FRAMEBUFFER_HEIGHT));
        b.to_async(tokio::runtime::Runtime::new().expect("Failed to start tokio runtime"))
            .iter(|| invoke_simple_implementation(input, &fb));
    });

    // c_group.bench_with_input("Assembler", &commands, |b, input| {
    //     let fb = Arc::new(FrameBuffer::new(FRAMEBUFFER_WIDTH, FRAMEBUFFER_HEIGHT));
    //     b.to_async(tokio::runtime::Runtime::new().expect("Failed to start tokio runtime"))
    //         .iter(|| invoke_assembler_implementation(input, &fb));
    // });
}

async fn invoke_simple_implementation(input: &[u8], fb: &Arc<FrameBuffer>) {
    let mut parser = SimpleParser::default();
    parser
        .parse(input, fb, DevNullTcpStream::default())
        .await
        .expect("Failed to parse commands");
}

async fn _invoke_assembler_implementation(input: &[u8], fb: &Arc<FrameBuffer>) {
    let mut parser = AssemblerParser::default();
    parser
        .parse(input, fb, DevNullTcpStream::default())
        .await
        .expect("Failed to parse commands");
}

criterion_group!(
    name = parsing;
    config = Criterion::default().warm_up_time(Duration::from_secs(3)).measurement_time(Duration::from_secs(5));
    targets = compare_implementations
);
criterion_main!(parsing);
