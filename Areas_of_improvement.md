## Areas of improvement in v0.2.0
1. **Scalabilty and performace**
Synchronous Scanning: The discover_tracks function runs on the main thread before the UI starts. If a user points this at a library with 50,000 songs, the app will hang at startup until the scan finishes.
Memory Usage: You load all Track structs into a Vec. While fine for 99% of users, this isn't optimized for massive libraries (no lazy loading or database).
2. **UI Virtualization (Processing Speed)**
The Problem: In draw_ui, we convert every single track in your library into a ListItem every time you draw.
code
Rust
// OLD CODE (Slow with 10k songs)
let items: Vec<ListItem> = player.tracks.iter().map(...).collect();
If you have 50,000 songs, you are allocating 50,000 UI widgets every frame, but only showing 20 lines.
The Fix: Manually slice the vector. Only generate list items for what fits on the screen.

3. Asynchronous Library Scanning (Startup Speed)
The Problem: When you run trix, the app freezes until it scans all folders. This feels "sluggish."
The Fix: Use std::thread and std::sync::mpsc::channel to load files in the background.

4. Optimize the Render Loop (CPU Usage)
The Problem: Currently, your app redraws the entire UI every 50ms (20 times a second), even if nothing changed. This eats CPU cycles.
The Fix: Only redraw when (A) an input event happens, or (B) one second has passed (to update the progress bar).

5. String Interning (Massive RAM Savings)
The Problem:
If you have 500 songs by "Daft Punk," you are currently storing the string "Daft Punk" 500 separate times in memory.
The Fix:
Use Arc<str> (Atomic Reference Counted string slice). This allows 500 tracks to point to the exact same memory location for the Artist name.
In src/main.rs:
code
Rust
use std::sync::Arc;

#[derive(Debug, Clone)]
struct Track {
    path: PathBuf,
    // If 10 tracks share the name "Unknown", they share the pointer
    display_name: Arc<str>, 
    // Add these to Track to avoid probing later
    artist: Option<Arc<str>>, 
    album: Option<Arc<str>>,
}

// When loading tracks, use a rudimentary cache
// (You would pass this cache map into your scanner function)
fn internalize(cache: &mut HashMap<String, Arc<str>>, val: String) -> Arc<str> {
    if let Some(existing) = cache.get(&val) {
        existing.clone()
    } else {
        let s: Arc<str> = val.into();
        cache.insert(val, s.clone());
        s
    }
}
Effect: RAM usage for metadata (Artists/Albums) drops by ~60-80% for typical libraries.
6. Reuse the Audio Sink (Latency/CPU)
The Problem:
Currently, inside start_track, you do:
code
Rust
let sink = Sink::try_new(&self.handle)...
Creating a Sink often involves talking to the OS audio daemon (PulseAudio/PipeWire/ALSA). Doing this every time a song changes causes a micro-stutter and unneeded syscalls.
The Fix:
Create the Sink once when the Player starts. When changing tracks, just clear the queue and append the new song.
code
Rust
struct Player {
    // Keep the sink alive forever
    sink: Sink, 
    // ...
}

impl Player {
    fn new(...) -> Result<Self> {
        // ...
        let sink = Sink::try_new(&handle)?; // Create ONCE
        // ...
    }

    fn start_track(&mut self, start_pos: Duration) -> Result<()> {
        // Stop whatever is playing and clear the buffer
        self.sink.stop(); 
        // Note: In rodio, stop() might invalidate the sink depending on version.
        // If stop() kills it, use: self.sink.clear(); (if available in your rodio version)
        // Or simply append a customized "Empty" source to flush it.
        
        // ... Load source ...
        
        // Push new song to existing sink
        self.sink.append(source);
        self.sink.play();
        
        Ok(())
    }
}
Effect: Playback starts instantly. No "pop" or delay between tracks.
7. Search Debouncing (Processing Efficiency)
The Problem:
If a user types "Metal" quickly, your current code filters the entire list 5 times: M -> Me -> Met -> Meta -> Metal.
The Fix:
Don't search immediately. Wait until the user stops typing for 150ms.
In src/main.rs:
code
Rust
struct UiState {
    // ...
    search_query: String,
    search_trigger: Option<Instant>, // New field
}

// In handle_key:
KeyCode::Char(c) => {
    ui.search_query.push(c);
    // Don't search yet, just set the timer
    ui.search_trigger = Some(Instant::now() + Duration::from_millis(150));
}

// In main loop (before draw):
if let Some(trigger) = ui.search_trigger {
    if Instant::now() >= trigger {
        apply_search_selection(player, &ui.search_query);
        ui.search_trigger = None;
        needs_redraw = true;
    }
}
Effect: Keeps the UI snappy even on slow CPUs when searching massive lists.
8. Binary Stripping (Disk Space & Startup)
The Problem:
Rust binaries contain debug symbols by default, making them large (10MB+). This slows down initial loading from disk.
The Fix:
Aggressively strip the binary in Cargo.toml.
In Cargo.toml:
code
Toml
[profile.release]
opt-level = 3
lto = "fat"       # "Fat" LTO optimizes across all crates (slow build, fast binary)
codegen-units = 1 # Reduces parallelism to allow better optimization
panic = "abort"   # Removes stack unwinding code (smaller binary)
strip = true      # Automatically strips symbols (requires Rust 1.59+)
Effect: Your binary size will likely drop from ~15MB to ~2MB. It will load into memory faster.
9. Normalized Search Key (CPU)
The Problem:
Currently, your search does:
code
Rust
t.display_name.to_ascii_lowercase().contains(&q)
This allocates a new String (to_ascii_lowercase) for every track every time you search. That's thousands of allocations per keystroke.
The Fix:
Store a "search key" in the Track struct when you load it.
code
Rust
struct Track {
    display_name: String,
    // Pre-computed lowercase version
    lower_name: String, 
}

// When loading:
let name = ...;
Track {
    display_name: name.clone(),
    lower_name: name.to_ascii_lowercase(), // Compute once!
}

// When searching:
// No allocation here! Just a slice check.
if t.lower_name.contains(&q) { ... }
Effect: Searching becomes "Zero Allocation." Extremely CPU efficient.

