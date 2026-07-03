use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use shogiesa_core::{Board, SideToMove, board::PieceType, sfen::Sfen};
use std::hint::black_box;

const STARTPOS: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";

fn bench_sfen_parse(c: &mut Criterion) {
    c.bench_function("sfen_parse", |b| {
        b.iter(|| Sfen::parse(black_box(STARTPOS)))
    });
}

fn bench_board_apply_normal(c: &mut Criterion) {
    c.bench_function("board_apply_normal", |b| {
        b.iter_batched(
            || Board::initial(SideToMove::Black),
            |mut board| {
                board
                    .apply_normal(SideToMove::Black, 7, 7, 7, 6, PieceType::Pawn)
                    .unwrap()
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_board_to_sfen(c: &mut Criterion) {
    let board = Board::initial(SideToMove::Black);
    c.bench_function("board_to_sfen", |b| b.iter(|| board.to_sfen()));
}

criterion_group!(
    benches,
    bench_sfen_parse,
    bench_board_apply_normal,
    bench_board_to_sfen
);
criterion_main!(benches);
