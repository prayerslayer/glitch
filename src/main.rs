extern crate rand;

use rand::distributions::{IndependentSample, Range};
use std::env;
use std::fs;
use std::fs::File;
use std::io::prelude::*;

fn get_random_u8(min: u8, max: u8) -> u8 {
    let between = Range::new(min, max);
    let mut rng = rand::thread_rng();
    return between.ind_sample(&mut rng);
}

fn get_random_u32(min: u32, max: u32) -> u32 {
    let between = Range::new(min, max);
    let mut rng = rand::thread_rng();
    return between.ind_sample(&mut rng);
}

#[derive(Copy, Clone, Debug)]
enum State {
    LOOKING_FOR_SOS,
    READING_HEADER_LENGTH,
    READING_ENTROPY,
    IDLE,
}

#[derive(Debug)]
enum OverwriteStrategy {
    RANDOM,
    RELATIVE_OFFSET,
}

struct Strategy {
    overwriteStrategy: OverwriteStrategy,
    maxOverwrites: u32,
    minOverwriteOffset: u8,
    maxOverwriteOffset: u8,
    minOverwriteGap: u32,
    maxOverwriteGap: u32,
}

fn strategy_to_str(strategy: &Strategy) -> Box<str> {
    let formatted = format!(
        "{:?}_{:0}_{:0}_{:0}",
        &strategy.overwriteStrategy,
        &strategy.maxOverwrites,
        &strategy.maxOverwriteOffset,
        &strategy.maxOverwriteGap
    );
    return formatted.into_boxed_str();
}

fn state_machine(state: State, b0: u8, b1: u8) -> State {
    match state {
        State::LOOKING_FOR_SOS => match (b0, b1) {
            (0xff, 0xda) => State::READING_HEADER_LENGTH,
            (_, _) => State::LOOKING_FOR_SOS,
        },
        State::READING_HEADER_LENGTH => match (b0, b1) {
            (0xff, 0xda) => State::READING_HEADER_LENGTH,
            (0xda, _) => State::READING_HEADER_LENGTH,
            (_, _) => State::READING_ENTROPY,
        },
        State::READING_ENTROPY => match (b0, b1) {
            (0xff, 0x00) => State::READING_ENTROPY,
            (0xff, _) => State::IDLE,
            (_, _) => State::READING_ENTROPY,
        },
        State::IDLE => State::IDLE,
    }
}

fn corrupt(filename: &str, strategy: Strategy) {
    let mut result: Vec<u8> = vec![];
    let f = File::open(filename).expect("file not found");

    let mut last_byte: Option<u8> = None;
    let mut next_overwrite: u64 = 0;
    let mut overwrite_count: u32 = 0;
    let mut state = State::LOOKING_FOR_SOS;
    let mut scan_header_length: u64 = 0;
    let mut scan_header_start: u64 = 0;
    let mut byte_index: u64 = 0;

    for current_byte in f.bytes() {
        byte_index += 1;
        match last_byte {
            None => match current_byte.ok() {
                Some(b) => {
                    last_byte = Some(b);
                    result.push(b);
                }
                None => panic!(),
            },
            Some(b0) => match current_byte.ok() {
                Some(b1) => {
                    last_byte = Some(b1);
                    let next_state = state_machine(state, b0, b1);
                    match (state, next_state) {
                        (State::LOOKING_FOR_SOS, State::LOOKING_FOR_SOS) => {}
                        (State::READING_ENTROPY, State::READING_ENTROPY) => {}
                        (State::READING_HEADER_LENGTH, State::READING_HEADER_LENGTH) => {}
                        (State::IDLE, _) => {}
                        (_1, _2) => println!("{:?}->{:?}: {:x} {:x}", _1, _2, b0, b1),
                    }
                    let nb: u8 = match (state, next_state) {
                        (State::READING_HEADER_LENGTH, _) => {
                            // FIXME assumes scan header length fewer than 256 bytes
                            scan_header_length = b1 as u64;
                            b1
                        }
                        (_, State::READING_HEADER_LENGTH) => {
                            scan_header_start = byte_index;
                            b1
                        }
                        (_, State::READING_ENTROPY) => {
                            let mut nb = b1;
                            if byte_index > scan_header_start + scan_header_length {
                                if next_overwrite == 0 {
                                    next_overwrite = byte_index
                                        + (get_random_u32(
                                            strategy.minOverwriteGap,
                                            strategy.maxOverwriteGap,
                                        ) as u64);
                                } else if byte_index == next_overwrite {
                                    if overwrite_count < strategy.maxOverwrites {
                                        next_overwrite = byte_index
                                            + (get_random_u32(
                                                strategy.minOverwriteGap,
                                                strategy.maxOverwriteGap,
                                            )
                                                as u64);
                                    }
                                    nb = match strategy.overwriteStrategy {
                                        OverwriteStrategy::RANDOM => get_random_u8(1, 255),
                                        OverwriteStrategy::RELATIVE_OFFSET => {
                                            let offset = get_random_u8(
                                                strategy.minOverwriteOffset,
                                                strategy.maxOverwriteOffset,
                                            );
                                            b1.checked_sub(offset).unwrap_or(b1)
                                        }
                                    };

                                    overwrite_count += 1;
                                }
                            }
                            nb
                        }
                        (_, _) => b1,
                    };
                    state = next_state;
                    result.push(nb)
                }
                None => panic!(),
            },
        }
    }

    let dir: &str = &format!("{}{}", filename, "-bad");
    fs::create_dir_all(dir).expect("cannot create dir");
    let target_filename = format!("{}/{}.jpg", dir, strategy_to_str(&strategy));
    let mut target = File::create(target_filename).expect("cannot create file");
    target.write_all(result.as_slice());
    target.sync_all();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let filename = &args[2];

    for max_overwrites_factor in 4..8u32 {
        // How many bytes to overwrite
        for max_overwrite_offset_factor in 2..5u8 {
            // How much to overwrite
            for offset_factor in 6..12u32 {
                // gap between overwrites
                let max_overwrites = 2u32.pow(max_overwrites_factor);
                let min_overwrite_offset = 1;
                let max_overwrite_offset = (2u8.pow(max_overwrite_offset_factor as u32) - 1) as u8;
                let min_overwrite_gap = 2u32.pow(offset_factor);
                let max_overwrite_gap = 2u32.pow(offset_factor + 1);
                let relative_strat = Strategy {
                    overwriteStrategy: OverwriteStrategy::RELATIVE_OFFSET,
                    maxOverwrites: max_overwrites,
                    minOverwriteOffset: min_overwrite_offset,
                    maxOverwriteOffset: max_overwrite_offset,
                    minOverwriteGap: min_overwrite_gap,
                    maxOverwriteGap: max_overwrite_gap,
                };
                let random_strat = Strategy {
                    overwriteStrategy: OverwriteStrategy::RANDOM,
                    ..relative_strat
                };
                corrupt(filename, random_strat);
                corrupt(filename, relative_strat);
            }
        }
    }
}
