//! Window geometry and verified placement, independent of the AX transport.
//! The transport must bind one original process/object identity and revalidate
//! it before each operation. A write status alone never proves placement.
use rustcast_core::Frame;

pub const CONSTRAINED: &str =
    "The app constrained or did not finish applying the window size or position. Try again.";
type Result<T> = std::result::Result<T, String>;

pub fn valid(frame: Frame) -> bool {
    [frame.x, frame.y, frame.width, frame.height]
        .iter()
        .all(|v| v.is_finite())
        && frame.width > 0.
        && frame.height > 0.
}
pub fn matches(actual: Frame, target: Frame) -> bool {
    valid(actual)
        && valid(target)
        && (actual.x - target.x).abs() < 8.
        && (actual.y - target.y).abs() < 8.
        && (actual.width - target.width).abs() < 8.
        && (actual.height - target.height).abs() < 8.
}
pub trait Transport {
    /// Includes deadline, cancellation, trust, original process start identity
    /// and original AX window membership checks. Never substitute another item.
    fn check(&mut self) -> Result<()>;
    fn read(&mut self) -> Result<Frame>;
    fn resize(&mut self, frame: Frame) -> Result<()>;
    fn position(&mut self, frame: Frame) -> Result<()>;
    fn pause(&mut self, seconds: f64) -> Result<()>;
}
pub fn apply(target: Frame, io: &mut impl Transport) -> Result<Frame> {
    if !valid(target) {
        return Err("Cannot apply an invalid window frame.".into());
    }
    for attempt in 0..3 {
        // Shrink before crossing displays, then resize again at the new origin:
        // macOS can clamp a taller window at its original position.
        io.check()?;
        io.resize(target)?;
        io.check()?;
        io.position(target)?;
        io.check()?;
        io.resize(target)?;
        io.check()?;
        let actual = io.read()?;
        io.check()?;
        if matches(actual, target) {
            return Ok(actual);
        }
        if attempt < 2 {
            io.pause(0.02)?;
        }
    }
    Err(CONSTRAINED.into())
}
pub fn screen_index(window: Frame, screens: &[Frame]) -> Result<usize> {
    if !valid(window) || screens.is_empty() || screens.iter().any(|f| !valid(*f)) {
        return Err("No valid screen available.".into());
    }
    let mut best = 0;
    let mut best_area = -1.;
    for (index, screen) in screens.iter().enumerate() {
        let width = (window.x + window.width).min(screen.x + screen.width) - window.x.max(screen.x);
        let height =
            (window.y + window.height).min(screen.y + screen.height) - window.y.max(screen.y);
        let area = width.max(0.) * height.max(0.);
        if area > best_area {
            best = index;
            best_area = area;
        }
    }
    Ok(best)
}
pub fn destination(window: Frame, screens: &[Frame], layout: &str) -> Result<Frame> {
    let index = screen_index(window, screens)?;
    if matches!(layout, "next-screen" | "previous-screen") {
        if screens.len() < 2 {
            return Err("Only one screen is connected.".into());
        }
        let next = (index
            + if layout == "next-screen" {
                1
            } else {
                screens.len() - 1
            })
            % screens.len();
        let (a, b) = (screens[index], screens[next]);
        let width = window.width.min(b.width);
        let height = window.height.min(b.height);
        Ok(Frame {
            x: (b.x + (window.x - a.x) / a.width * b.width)
                .max(b.x)
                .min(b.x + b.width - width),
            y: (b.y + (window.y - a.y) / a.height * b.height)
                .max(b.y)
                .min(b.y + b.height - height),
            width,
            height,
        })
    } else {
        rustcast_core::layout(screens[index], layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(x: f64, y: f64, width: f64, height: f64) -> Frame {
        Frame {
            x,
            y,
            width,
            height,
        }
    }
    struct Io {
        calls: Vec<&'static str>,
        result: Frame,
        checks: usize,
        revoke_at: usize,
        reads_before_match: usize,
    }
    impl Transport for Io {
        fn check(&mut self) -> Result<()> {
            self.checks += 1;
            if self.checks == self.revoke_at {
                Err("owned identity expired".into())
            } else {
                Ok(())
            }
        }
        fn read(&mut self) -> Result<Frame> {
            self.calls.push("read");
            if self.reads_before_match > 0 {
                self.reads_before_match -= 1;
                Ok(frame(0., 0., 200., 100.))
            } else {
                Ok(self.result)
            }
        }
        fn resize(&mut self, _: Frame) -> Result<()> {
            self.calls.push("resize");
            Ok(())
        }
        fn position(&mut self, _: Frame) -> Result<()> {
            self.calls.push("position");
            Ok(())
        }
        fn pause(&mut self, seconds: f64) -> Result<()> {
            assert_eq!(seconds, 0.02);
            self.calls.push("pause");
            Ok(())
        }
    }
    fn io(reads: usize) -> Io {
        Io {
            calls: vec![],
            result: frame(20., 30., 800., 700.),
            checks: 0,
            revoke_at: usize::MAX,
            reads_before_match: reads,
        }
    }
    #[test]
    fn applies_resize_position_resize_and_verifies_actual_geometry_with_bounded_retries() {
        let mut io = io(2);
        let result = apply(io.result, &mut io).unwrap();
        assert!(matches(result, io.result));
        assert_eq!(
            io.calls,
            [
                "resize", "position", "resize", "read", "pause", "resize", "position", "resize",
                "read", "pause", "resize", "position", "resize", "read"
            ]
        );
        let mut constrained = self::io(3);
        assert_eq!(
            apply(constrained.result, &mut constrained).unwrap_err(),
            CONSTRAINED
        );
        assert_eq!(
            constrained.calls.iter().filter(|c| **c == "read").count(),
            3
        );
    }
    #[test]
    fn revoked_identity_stops_before_next_native_effect() {
        for (at, expected) in [
            (1, vec![]),
            (2, vec!["resize"]),
            (3, vec!["resize", "position"]),
            (4, vec!["resize", "position", "resize"]),
            (5, vec!["resize", "position", "resize", "read"]),
        ] {
            let mut io = io(0);
            io.revoke_at = at;
            assert!(apply(io.result, &mut io).is_err());
            assert_eq!(io.calls, expected);
        }
    }
    #[test]
    fn overlap_and_screen_transfer_keep_relative_position_and_clamp_size() {
        let screens = [frame(0., 0., 1000., 800.), frame(1000., -200., 500., 600.)];
        let window = frame(400., 200., 900., 700.);
        assert_eq!(screen_index(window, &screens).unwrap(), 0);
        let next = destination(window, &screens, "next-screen").unwrap();
        assert!(matches(next, frame(1000., -200., 500., 600.)));
        let previous =
            destination(frame(1100., -100., 200., 300.), &screens, "previous-screen").unwrap();
        assert!(matches(previous, frame(200., 800. / 6., 200., 300.)));
        assert!(destination(window, &screens[..1], "next-screen")
            .unwrap_err()
            .contains("Only one"));
    }
    #[test]
    fn placement_tolerance_is_strict_and_nonfinite_or_zero_geometry_is_rejected() {
        let target = frame(0., 0., 100., 100.);
        assert!(matches(frame(7.9, 7.9, 107.9, 107.9), target));
        assert!(!matches(frame(8., 0., 100., 100.), target));
        assert!(!matches(frame(f64::NAN, 0., 100., 100.), target));
        assert!(destination(target, &[frame(0., 0., 0., 800.)], "left-half").is_err());
        let mut io = io(0);
        assert!(apply(frame(0., 0., f64::INFINITY, 100.), &mut io).is_err());
        assert!(io.calls.is_empty());
    }
}
