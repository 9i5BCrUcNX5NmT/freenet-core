#[derive(Debug, PartialEq)]
pub struct Segment {
    depth: u8,
    position: u8,
}

impl Segment {
    pub fn new(depth: u8, position: u8) -> Result<Segment, &'static str> {
        if depth == 0 && position != 0 {
            Err("Invalid position for depth 0")
        } else if depth != 0 && position >= 2u8.pow(depth as u32) {
            Err("Invalid position for given depth")
        } else {
            Ok(Segment { depth, position })
        }
    }

    pub fn enclosing_segment(location: f64, depth: u8) -> Result<Segment, &'static str> {
        if !(0.0..=1.0).contains(&location) {
            Err("Invalid location. Should be between 0.0 and 1.0")
        } else {
            let position = (location * (2u8.pow(depth as u32) as f64)).floor() as u8;
            Segment::new(depth, position)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        assert_eq!(Segment::new(0, 0), Ok(Segment { depth: 0, position: 0 }));
        assert_eq!(Segment::new(1, 0), Ok(Segment { depth: 1, position: 0 }));
        assert_eq!(Segment::new(1, 1), Ok(Segment { depth: 1, position: 1 }));
        assert_eq!(Segment::new(2, 3), Ok(Segment { depth: 2, position: 3 }));
        assert_eq!(Segment::new(0, 1), Err("Invalid position for depth 0"));
        assert_eq!(Segment::new(1, 2), Err("Invalid position for given depth"));
    }

    #[test]
    fn test_enclosing_segment() {
        assert_eq!(Segment::enclosing_segment(0.0, 0), Ok(Segment { depth: 0, position: 0 }));
        assert_eq!(Segment::enclosing_segment(0.25, 2), Ok(Segment { depth: 2, position: 1 }));
        assert_eq!(Segment::enclosing_segment(0.75, 2), Ok(Segment { depth: 2, position: 3 }));
        assert_eq!(Segment::enclosing_segment(1.1, 1), Err("Invalid location. Should be between 0.0 and 1.0"));
    }
}
