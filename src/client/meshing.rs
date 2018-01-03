use std::sync::mpsc::{Sender, Receiver};
use ::core::messages::client::{ToInput, ToMeshing};
use ::block::{BlockRegistry, Chunk, ChunkPos};
use std::collections::HashMap;
use std::cell::RefCell;
use std::sync::Arc;
use ::{CHUNK_SIZE};
use std;

enum ChunkState {
    Received(usize, Chunk),
    Meshed(Chunk),
}

type ChunkMap = HashMap<ChunkPos, RefCell<ChunkState>>;

const ADJ_CHUNKS: [[i64; 3]; 6] = [
    [ 0,  0, -1],
    [ 0,  0,  1],
    [ 1,  0,  0],
    [-1,  0,  0],
    [ 0,  1,  0],
    [ 0, -1,  0],
];

pub fn start(rx: Receiver<ToMeshing>, input_tx: Sender<ToInput>, block_registry: Arc<BlockRegistry>) {
    let mut chunks: ChunkMap = HashMap::new();
    let mut dummy = Chunk {
        blocks: Box::new([[[::block::BlockId::from(0); CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE]),
        sides: Box::new([[[0xFF; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE]),
    };
    let mut dropping = false;

    loop {
        //println!("Meshing: new loop run");

        // Receive all pending updates
        match rx.recv() {
            Ok(message) => handle_message(&mut chunks, message, &mut dropping),
            Err(_) => return,
        }

        while let Ok(message) = rx.try_recv() {
            handle_message(&mut chunks, message, &mut dropping);
        }

        // Update chunks
        'all_chunks: for (pos, state) in &chunks {
            if let ChunkState::Received(fragment_count, ref mut chunk) = *state.borrow_mut() {
                //println!("fragment_count {} @ pos {:?}", fragment_count, pos);
                if fragment_count < CHUNK_SIZE*CHUNK_SIZE {
                    continue;
                }
                for &adj in &ADJ_CHUNKS {
                    match chunks.get(&ChunkPos(pos.0 + adj[0], pos.1 + adj[1], pos.2 + adj[2])) {
                        Some(ref cell) => {
                            if let ChunkState::Received(fragment_count, _) = *cell.borrow() {
                                //println!("--> adj @ {:?} has {} fragments", &ChunkPos(pos.0 + adj[0], pos.1 + adj[1], pos.2 + adj[2]), fragment_count);
                                if fragment_count < CHUNK_SIZE*CHUNK_SIZE {
                                    continue 'all_chunks;
                                }
                            }
                        },
                        _ => continue 'all_chunks,
                    }
                }

                println!("Meshing: rendering chunk @ {:?}", pos);

                {
                    let sides = &mut chunk.sides;
                    let blocks = &chunk.blocks;
                    for side in 0..6 {
                        let adj = ADJ_CHUNKS[side];
                        match *chunks.get(&ChunkPos(pos.0 + adj[0], pos.1 + adj[1], pos.2 + adj[2])).unwrap().borrow() {
                            ChunkState::Received(_, ref c) => for (i1, i2) in get_range(adj[0], false).zip(get_range(adj[0], true)) {
                                for (j1, j2) in get_range(adj[1], false).zip(get_range(adj[1], true)) {
                                    for (k1, k2) in get_range(adj[2], false).zip(get_range(adj[2], true)) {
                                        if !block_registry.get_block(c.blocks[i1][j1][k1]).is_opaque() {
                                            sides[i2][j2][k2] ^= 1 << side;
                                        }
                                    }
                                }
                            },
                            ChunkState::Meshed(ref c) => for (i1, i2) in get_range(adj[0], false).zip(get_range(adj[0], true)) {
                                for (j1, j2) in get_range(adj[1], false).zip(get_range(adj[1], true)) {
                                    for (k1, k2) in get_range(adj[2], false).zip(get_range(adj[2], true)) {
                                        if !block_registry.get_block(c.blocks[i1][j1][k1]).is_opaque() {
                                            sides[i2][j2][k2] ^= 1 << side;
                                        }
                                    }
                                }
                            },
                        }
                    }
                    let sz = CHUNK_SIZE as i64;
                    for i in 0..sz {
                        for j in 0..sz {
                            for k in 0..sz {
                                for side in 0..6 {
                                    let adj = ADJ_CHUNKS[side];
                                    let (x, y, z) = (i + adj[0], j + adj[1], k + adj[2]);
                                    if 0 <= x && x < sz && 0 <= y && y < sz && 0 <= z && z < sz {
                                        if !block_registry.get_block(blocks[x as usize][y as usize][z as usize]).is_opaque() {
                                            sides[i as usize][j as usize][k as usize] ^= 1 << side;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                input_tx.send(ToInput::NewChunkBuffer(pos.clone(), chunk.calculate_mesh(&block_registry))).unwrap();
                println!("Meshing: updated chunk @ {:?}", pos);

                ::std::mem::swap(&mut dummy, chunk);
            }
            else {
                continue;
            }

            let mut new_state = ChunkState::Meshed(dummy);
            ::std::mem::swap(&mut new_state, &mut *state.borrow_mut());
            if let ChunkState::Received(_, dummy_chunks) = new_state {
                dummy = dummy_chunks;
            }
            else {
                unreachable!();
            }
        }
    }
}

fn handle_message(chunks: &mut ChunkMap, message: ToMeshing, dropping: &mut bool) {
    match message {
        ToMeshing::AllowChunk(pos) => {
            *dropping = false;
            println!("Meshing: allowed chunk @ {:?}", pos);
            chunks.insert(pos, RefCell::new(ChunkState::Received(0, Chunk {
                blocks: Box::new([[[::block::BlockId::from(0); CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE]),
                sides: Box::new([[[0xFF; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE]),
            })));
        },
        // TODO: Ensure the chunk has been allowed
        ToMeshing::NewChunkFragment(pos, fpos, frag) => {
            if let Some(state) = chunks.get(&pos) {
                *dropping = false;
                if let ChunkState::Received(ref mut fragment_count, ref mut chunk) = *state.borrow_mut() {
                    *fragment_count += 1;
                    if *fragment_count == CHUNK_SIZE*CHUNK_SIZE {
                        println!("Meshing: new full chunk !");
                    }
                    chunk.blocks[fpos.0][fpos.1] = (*frag).clone();
                }
            }
            else if !*dropping {
                println!("[WARNING] Meshing: dropped ChunkFragment because it was not allowed!");
                *dropping = true;
            }
        },
        ToMeshing::RemoveChunk(pos) => {
            *dropping = false;
            println!("Meshing: removed chunk @ {:?}", pos);
            chunks.remove(&pos);
        },
    }
    //println!("Meshing: processed message");
}

fn get_range(x: i64, reversed: bool) -> std::ops::Range<usize> {
    match x {
        0 => 0..CHUNK_SIZE,
        1 => if reversed { (CHUNK_SIZE-1)..CHUNK_SIZE } else { 0..1 },
        -1 => if reversed { 0..1 } else { (CHUNK_SIZE-1)..CHUNK_SIZE },
        _ => panic!("Impossible value"),
    }
}
