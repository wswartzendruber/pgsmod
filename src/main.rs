/*
 * Copyright 2020 William Swartzendruber
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a
 * copy of the MPL was not distributed with this file, You can obtain one at
 * https://mozilla.org/MPL/2.0/.
 */

mod pgs;
mod rgb;

use pgs::{
    Seg,
    SegBody,
    read::{ReadSegExt, SegReadError},
    write::WriteSegExt,
};
use std::{
    collections::HashMap,
    fs::File,
    io::{stdin, stdout, BufReader, BufWriter, ErrorKind, Read, Write},
};
use clap::{crate_version, Arg, App};

#[derive(Eq, Hash, PartialEq)]
struct ObjHandle {
    comp_num: u16,
    obj_id: u16,
}

#[derive(Clone, Copy, PartialEq)]
struct Size {
    width: u16,
    height: u16,
}

fn main() {

    let matches = App::new("PGSMod")
        .version(crate_version!())
        .about("Modifies PGS subtitles")
        .arg(Arg::with_name("crop-width")
            .long("crop-width")
            .short("w")
            .value_name("PIXELS")
            .help("Width to crop each subtitle frame to")
            .takes_value(true)
            .required(true)
            .validator(|value| {
                if value.parse::<usize>().is_ok() {
                    Ok(())
                } else {
                    Err("must be an unsigned integer".to_string())
                }
            })
        )
        .arg(Arg::with_name("crop-height")
            .long("crop-height")
            .short("h")
            .value_name("PIXELS")
            .help("Height to crop each subtitle frame to")
            .takes_value(true)
            .required(true)
            .validator(|value| {
                if value.parse::<usize>().is_ok() {
                    Ok(())
                } else {
                    Err("must be an unsigned integer".to_string())
                }
            })
        )
        .arg(Arg::with_name("margin")
            .long("margin")
            .short("m")
            .value_name("PIXELS")
            .help("Minimum margin around the screen border to enforce")
            .takes_value(true)
            .required(false)
            .default_value("30")
            .validator(|value| {
                if value.parse::<usize>().is_ok() {
                    Ok(())
                } else {
                    Err("must be an unsigned integer".to_string())
                }
            })
        )
        .arg(Arg::with_name("input")
            .index(1)
            .value_name("INPUT-FILE")
            .help("Input PGS file; use - for STDIN")
            .required(true)
        )
        .arg(Arg::with_name("output")
            .index(2)
            .value_name("OUTPUT-FILE")
            .help("Output PGS file; use - for STDOUT")
            .required(true)
        )
        .after_help("This utility will crop PGS subtitles found in Blu-ray discs so that they \
            can match any cropping that has been done to the main video stream, thereby \
            preventing the subtitles from appearing squished or distorted by the player.")
        .get_matches();
    let crop_width = matches.value_of("crop-width").unwrap().parse::<u16>().unwrap();
    let crop_height = matches.value_of("crop-height").unwrap().parse::<u16>().unwrap();
    let margin = matches.value_of("margin").unwrap().parse::<u16>().unwrap();
    let input_value = matches.value_of("input").unwrap();
    let (mut stdin_read, mut file_read);
    let mut input = BufReader::<&mut dyn Read>::new(
        if input_value == "-" {
            stdin_read = stdin();
            &mut stdin_read
        } else {
            file_read = File::open(input_value)
                .expect("Could not open input file for writing.");
            &mut file_read
        }
    );
    let output_value = matches.value_of("output").unwrap();
    let (mut stdout_write, mut file_write);
    let mut output = BufWriter::<&mut dyn Write>::new(
        if output_value == "-" {
            stdout_write = stdout();
            &mut stdout_write
        } else {
            file_write = File::create(output_value)
                .expect("Could not open output file for writing.");
            &mut file_write
        }
    );
    let mut segs = Vec::<Seg>::new();

    eprintln!("Reading PGS segments into memory...");

    loop {

        let seg = match input.read_seg() {
            Ok(seg) => {
                seg
            }
            Err(err) => {
                match err {
                    SegReadError::IoError { source } => {
                        if source.kind() != ErrorKind::UnexpectedEof {
                            panic!("Could not read segment due to IO error: {}", source)
                        }
                    }
                    _ => panic!("Could not read segment due to bitstream error: {}", err)
                }
                break
            }
        };

        segs.push(seg);
    }

    let mut comp_num = 0;
    let mut obj_sizes = HashMap::new();

    eprintln!("Inventorying segments...");

    for seg in segs.iter() {
        match &seg.body {
            SegBody::PresComp(pcs) => {
                comp_num = pcs.comp_num
            }
            SegBody::ObjDef(ods) => {
                if obj_sizes.insert(
                    ObjHandle { comp_num, obj_id: ods.id },
                    Size { width: ods.width, height: ods.height },
                ).is_some() {
                    panic!("Duplicate object ID detected in a given display set.")
                }
            }
            _ => {
                ()
            }
        }
    }

    let mut screen_sizes = Vec::<Size>::new();
    let mut screen_full_size = Size { width: 0, height: 0 };

    eprintln!("Performing modifications...");

    for seg in segs.iter_mut() {
        match &mut seg.body {
            SegBody::PresComp(pcs) => {
                comp_num = pcs.comp_num;
                screen_full_size = Size { width: pcs.width, height: pcs.height };
                if !screen_sizes.contains(&screen_full_size) {
                    eprintln!(
                        "New resolution encountered: {}x{}",
                        screen_full_size.width, screen_full_size.height,
                    );
                    screen_sizes.push(screen_full_size);
                }
                for comp_obj in pcs.comp_objs.iter_mut() {
                    let obj_size = obj_sizes.get(
                        &ObjHandle { comp_num, obj_id: comp_obj.obj_id }
                    ).expect("Could not find object size.");
                    comp_obj.x = cropped_object_offset(
                        screen_full_size.width,
                        crop_width,
                        obj_size.width,
                        comp_obj.x,
                        margin,
                    );
                    comp_obj.y = cropped_object_offset(
                        screen_full_size.height,
                        crop_height,
                        obj_size.height,
                        comp_obj.y,
                        margin,
                    );
                    match &mut comp_obj.crop {
                        Some(crop) => {
                            crop.x = cropped_object_offset(
                                screen_full_size.width,
                                crop_width,
                                crop.width,
                                crop.x,
                                margin,
                            );
                            crop.y = cropped_object_offset(
                                screen_full_size.height,
                                crop_height,
                                crop.height,
                                crop.y,
                                margin,
                            );
                        }
                        None => {
                            ()
                        }
                    }
                }
                pcs.width = crop_width;
                pcs.height = crop_height;
            }
            SegBody::WinDef(wds) => {
                for wd in wds.iter_mut() {
                    wd.x = cropped_object_offset(
                        screen_full_size.width,
                        crop_width,
                        wd.width,
                        wd.x,
                        margin,
                    );
                    wd.y = cropped_object_offset(
                        screen_full_size.height,
                        crop_height,
                        wd.height,
                        wd.y,
                        margin,
                    );
                }
            }
            _ => ()
        }
    }

    eprintln!("Writing modified segments...");

    for seg in segs {
        if let Err(err) = output.write_seg(&seg) {
            panic!("Could not write frame to output stream: {:?}", err)
        }
    }

    output.flush().expect("Could not flush output stream.");
}

fn cropped_object_offset(
    screen_full_size: u16,
    screen_crop_size: u16,
    object_size: u16,
    object_offset: u16,
    margin: u16,
) -> u16 {

    if object_size + 2 * margin > screen_crop_size {
        eprintln!("WARNING: Object or window cannot fit within new margins.");
        return 0
    }

    let new_offset = object_offset - (screen_full_size - screen_crop_size) / 2;

    match new_offset {
        o if o < margin =>
            margin,
        o if o + object_size + margin > screen_crop_size =>
            screen_crop_size - object_size - margin,
        _ =>
            new_offset,
    }
}
