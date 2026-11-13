//! The render thread. Flipping a page blocks until the next vsync and decoding a picture takes
//! milliseconds, none of which may delay a key press: the input thread only leaves its newest
//! frame here. A frame that is not picked up in time is replaced, so rendering never falls behind.

use crate::compose::{Compositor, Frame, Stale};
use crate::fb::Fb;
use crate::render::Renderer;
use crate::Log;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Mail {
    frame: Option<Frame>,
    stale: Vec<Stale>,
    rendering: bool,
}

pub struct Display {
    mail: Arc<(Mutex<Mail>, Condvar)>,
}

impl Display {
    pub fn start(renderer: Renderer, fb: Option<Fb>, log: Log) -> Display {
        let mail = Arc::new((Mutex::new(Mail::default()), Condvar::new()));
        let shared = mail.clone();
        std::thread::spawn(move || run(shared, Compositor::new(renderer), fb, log));
        Display { mail }
    }

    /// Replace the frame waiting to be drawn.
    pub fn show(&self, f: Frame) {
        self.mail.0.lock().unwrap().frame = Some(f);
        self.mail.1.notify_all();
    }

    pub fn stale(&self, s: Stale) {
        self.mail.0.lock().unwrap().stale.push(s);
    }

    /// Wait until everything handed over is on the screen.
    pub fn wait_idle(&self) {
        let mut m = self.mail.0.lock().unwrap();
        while m.frame.is_some() || m.rendering {
            m = self.mail.1.wait(m).unwrap();
        }
    }
}

/// Frame times since the last report.
#[derive(Default)]
struct Stats {
    frames: u32,
    draw: Duration,
    draw_max: Duration,
    blit: Duration,
    blit_max: Duration,
}

impl Stats {
    fn report(&self, seconds: f32) -> String {
        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
        let n = self.frames.max(1) as f32;
        format!("perf: {:.1} fps, draw {:.1}/{:.1} ms, blit {:.1}/{:.1} ms (avg/max)", self.frames as f32 / seconds, ms(self.draw) / n, ms(self.draw_max), ms(self.blit) / n, ms(self.blit_max))
    }
}

fn run(mail: Arc<(Mutex<Mail>, Condvar)>, mut comp: Compositor, mut fb: Option<Fb>, log: Log) {
    let mut stats = Stats::default();
    let mut since = Instant::now();
    loop {
        let (frame, stale) = {
            let mut m = mail.0.lock().unwrap();
            m.rendering = false;
            mail.1.notify_all();
            while m.frame.is_none() {
                m = mail.1.wait(m).unwrap();
            }
            m.rendering = true;
            (m.frame.take().unwrap(), std::mem::take(&mut m.stale))
        };
        stale.into_iter().for_each(|s| comp.stale(s));
        let t0 = Instant::now();
        let (canvas, dirty) = comp.compose(&frame);
        let t1 = Instant::now();
        let changed = fb.as_mut().map(|fb| fb.blit(canvas, &dirty)).unwrap_or(false);
        let t2 = Instant::now();
        if !changed {
            continue;
        }
        if let Some(fb) = fb.as_mut() {
            fb.flip();
        }
        stats.frames += 1;
        stats.draw += t1 - t0;
        stats.blit += t2 - t1;
        stats.draw_max = stats.draw_max.max(t1 - t0);
        stats.blit_max = stats.blit_max.max(t2 - t1);
        if since.elapsed() >= Duration::from_secs(10) {
            log.trace(&stats.report(since.elapsed().as_secs_f32()));
            stats = Stats::default();
            since = Instant::now();
        }
    }
}
