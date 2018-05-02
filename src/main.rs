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

fn get_random_u64(min: u64, max: u64) -> u64 {
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

#[derive(Debug)]
enum PlacementStrategy {
    CONSTANT,
    RANDOM,
}

struct Strategy {
    placementStrategy: PlacementStrategy,
    overwriteStrategy: OverwriteStrategy,
    numBytesToOverwrite: u32,
    minOverwriteOffset: u8,
    maxOverwriteOffset: u8,
}

fn strategy_to_str(strategy: &Strategy) -> Box<str> {
    let formatted = format!(
        "{:?}_{:?}_{:0}_{:0}",
        &strategy.placementStrategy,
        &strategy.overwriteStrategy,
        &strategy.numBytesToOverwrite,
        &strategy.maxOverwriteOffset,
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
            (0xff, _) => State::LOOKING_FOR_SOS,
            (_, _) => State::READING_ENTROPY,
        },
        State::IDLE => State::IDLE,
    }
}

fn get_overwrites(scan_start: u64, scan_end: u64, strategy: &Strategy) -> Vec<u64> {
    let mut result: Vec<u64> = vec![];
    match strategy.placementStrategy {
        PlacementStrategy::CONSTANT => {
            let interval = (scan_end - scan_start) / (strategy.numBytesToOverwrite as u64);
            let mut byte_index = scan_start;
            while byte_index < scan_end {
                byte_index = byte_index + interval;
                result.push(byte_index)
            }
        }
        PlacementStrategy::RANDOM => for _ in 0..strategy.numBytesToOverwrite {
            let mut index = get_random_u64(scan_start, scan_end);
            while result.contains(&index) {
                index = get_random_u64(scan_start, scan_end);
            }
            result.push(index);
        },
    }
    return result;
}

fn corrupt_range(content: &Vec<u8>, start: u64, end: u64, strategy: &Strategy) -> Vec<u8> {
    let indexes_to_overwrite: Vec<u64> = get_overwrites(start, end, &strategy);
    let mut byte_index: u64 = 0;
    let mut result: Vec<u8> = vec![];

    println!("corrupting scan: {:0}..{:0}", start, end);

    for b1 in content {
        let mut nb: u8 = *b1;

        if byte_index > start && byte_index < end && indexes_to_overwrite.contains(&byte_index) {
            nb = match strategy.overwriteStrategy {
                OverwriteStrategy::RANDOM => get_random_u8(1, 255),
                OverwriteStrategy::RELATIVE_OFFSET => {
                    let offset =
                        get_random_u8(strategy.minOverwriteOffset, strategy.maxOverwriteOffset);
                    b1.checked_sub(offset).unwrap_or(*b1)
                }
            };
        }

        byte_index += 1;
        result.push(nb);
    }

    return result;
}

fn find_scans(content: &Vec<u8>) -> Vec<(u64, u64)> {
    let mut last_byte: Option<u8> = None;
    let mut state = State::LOOKING_FOR_SOS;
    let mut scan_header_length: u64 = 0;
    let mut scan_header_start: u64 = 0;
    let mut byte_index: u64 = 0;
    let mut scans: Vec<(u64, u64)> = vec![];
    let mut scan_start = 0;

    for current_byte in content {
        byte_index += 1;

        match last_byte {
            None => last_byte = Some(*current_byte),
            Some(b0) => {
                let b1 = current_byte;
                last_byte = Some(*b1);
                let next_state = state_machine(state, b0, *b1);

                // Debug print
                match (state, next_state) {
                    (State::LOOKING_FOR_SOS, State::LOOKING_FOR_SOS) => {}
                    (State::READING_ENTROPY, State::READING_ENTROPY) => {}
                    (State::READING_HEADER_LENGTH, State::READING_HEADER_LENGTH) => {}
                    (State::IDLE, State::IDLE) => {}
                    (_1, _2) => println!("{:?}->{:?}: {:x} {:x}", _1, _2, b0, b1),
                };

                match (state, next_state) {
                    (State::READING_HEADER_LENGTH, _) => {
                        // FIXME assumes scan header length fewer than 256 bytes
                        scan_header_length = *b1 as u64;
                        scan_start = 0;
                    }
                    // Case for single scan
                    (State::READING_ENTROPY, State::IDLE) => {
                        scans.push((scan_start, byte_index - 1))
                    }
                    // Case for multiple scans
                    (State::READING_ENTROPY, State::LOOKING_FOR_SOS) => {
                        scans.push((scan_start, byte_index - 1))
                    }
                    (_, State::READING_HEADER_LENGTH) => {
                        scan_header_start = byte_index;
                    }
                    (_, State::READING_ENTROPY) => {
                        if byte_index == scan_header_start + scan_header_length + 1 {
                            scan_start = byte_index;
                        }
                    }
                    (_, _) => {}
                }
                state = next_state;
            }
        }
    }
    return scans;
}

fn corrupt(filename: &str, strategy: Strategy) {
    let f = File::open(filename).expect("file not found");
    let mut result: Vec<u8> = f.bytes().map(|b| b.unwrap()).collect();
    let scans = find_scans(&result);

    for (start, end) in scans {
        result = corrupt_range(&result, start, end, &strategy);
    }

    let dir: &str = &format!("{}{}", filename, "-bad");
    fs::create_dir_all(dir).expect("cannot create dir");
    let target_filename = format!("{}/{}.jpg", dir, strategy_to_str(&strategy));
    write_to_disk(result, &target_filename);
}

fn write_to_disk(bytes: Vec<u8>, filename: &str) {
    let mut target = File::create(filename).expect("cannot create file");
    target.write_all(bytes.as_slice());
    target.sync_all();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let filename = match args.last() {
        Some(s) => s,
        None => panic!("Filename argument is needed")
    };

    // How many bytes to overwrite
    for num_overwrites in 4..10u32 {
        // How much to overwrite
        for max_overwrite_offset_factor in 2..5u8 {
            let min_overwrite_offset = 1;
            let max_overwrite_offset = (2u8.pow(max_overwrite_offset_factor as u32) - 1) as u8;
            let relative_strat = Strategy {
                placementStrategy: PlacementStrategy::RANDOM,
                overwriteStrategy: OverwriteStrategy::RELATIVE_OFFSET,
                numBytesToOverwrite: 2u32.pow(num_overwrites),
                minOverwriteOffset: min_overwrite_offset,
                maxOverwriteOffset: max_overwrite_offset,
            };
            let random_strat = Strategy {
                placementStrategy: PlacementStrategy::RANDOM,
                overwriteStrategy: OverwriteStrategy::RANDOM,
                ..relative_strat
            };
            corrupt(filename, relative_strat);
            corrupt(filename, random_strat);
        }
    }
}
