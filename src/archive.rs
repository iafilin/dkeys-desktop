#[cfg(target_os = "macos")]
pub mod macos;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SegmentColor {
    Recording,
    Placeholder,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub start: f64,
    pub duration: f64,
    pub color: SegmentColor,
}

#[derive(Clone, Debug)]
pub struct ArchivePlaylist {
    pub segments: Vec<Segment>,
    pub duration: f64,
}

pub fn parse_m3u8(text: &str) -> ArchivePlaylist {
    let mut segments = Vec::new();
    let mut cursor = 0.0_f64;
    let mut pending_color = None;
    let mut pending_duration = None;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("#EXT-X-COLOR:") {
            pending_color = Some(match rest.trim() {
                "Red" => SegmentColor::Recording,
                "Black" => SegmentColor::Placeholder,
                _ => SegmentColor::Unknown,
            });
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            pending_duration = rest
                .split(',')
                .next()
                .and_then(|value| value.trim().parse::<f64>().ok());
        } else if !line.is_empty()
            && !line.starts_with('#')
            && let Some(duration) = pending_duration.take()
        {
            segments.push(Segment {
                start: cursor,
                duration,
                color: pending_color.take().unwrap_or(SegmentColor::Unknown),
            });
            cursor += duration;
        }
    }
    ArchivePlaylist {
        segments,
        duration: cursor,
    }
}

#[derive(Clone, Debug)]
pub enum PlayerEvent {
    State {
        position: f64,
        duration: f64,
        paused: bool,
        finished: bool,
    },
    Error(String),
}

pub trait ArchiveControl: Send {
    fn play(&mut self);
    fn pause(&mut self);
    fn seek(&mut self, position: f64);
    fn set_speed(&mut self, speed: f32);
}

pub fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        (total / 60) % 60,
        total % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_segments_with_colors() {
        let playlist = parse_m3u8(
            "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-PLAYLIST-TYPE:VOD\n\
             #EXT-X-DISCONTINUITY\n#EXT-X-COLOR:Black\n#EXTINF:4.000,\nseg-0\n\
             #EXT-X-DISCONTINUITY\n#EXT-X-COLOR:Red\n#EXTINF:15.000,\nseg-1\n\
             #EXT-X-COLOR:Red\n#EXTINF:16.000,\nseg-2\n",
        );
        assert_eq!(playlist.segments.len(), 3);
        assert_eq!(playlist.segments[0].color, SegmentColor::Placeholder);
        assert_eq!(playlist.segments[1].color, SegmentColor::Recording);
        assert!((playlist.segments[1].start - 4.0).abs() < f64::EPSILON);
        assert!((playlist.duration - 35.0).abs() < f64::EPSILON);
    }

    #[test]
    fn formats_time() {
        assert_eq!(format_time(0.0), "00:00:00");
        assert_eq!(format_time(3661.4), "01:01:01");
    }
}
