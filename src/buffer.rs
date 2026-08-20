/// A fixed-size circular buffer for tracking recent keystrokes.
/// 
/// Used by the keyboard hook to maintain a rolling window of typed characters
/// for matching against snippet triggers. Zero heap allocation after construction.
pub struct KeyBuffer {
    buffer: Vec<char>,
    capacity: usize,
    head: usize,   // Points to the next write position
    len: usize,    // Current number of valid chars
}

impl KeyBuffer {
    /// Creates a new buffer with the specified capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec!['\0'; capacity],
            capacity,
            head: 0,
            len: 0,
        }
    }

    /// Appends a character, wrapping if full (oldest char lost).
    pub fn push(&mut self, ch: char) {
        if self.capacity == 0 {
            return;
        }
        self.buffer[self.head] = ch;
        self.head = (self.head + 1) % self.capacity;
        if self.len < self.capacity {
            self.len += 1;
        }
    }

    /// Removes and returns the most recent character.
    pub fn pop(&mut self) -> Option<char> {
        if self.len == 0 {
            return None;
        }
        self.head = if self.head == 0 {
            self.capacity - 1
        } else {
            self.head - 1
        };
        self.len -= 1;
        Some(self.buffer[self.head])
    }

    /// Empties the buffer.
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// Returns the current number of characters in the buffer.
    #[must_use]
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the buffer is empty.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns true if the buffer content ends with the given trigger string.
    /// 
    /// Optimized for zero heap allocations and instant early-exit on mismatch.
    #[must_use]
    #[inline]
    pub fn ends_with(&self, trigger: &str) -> bool {
        if trigger.is_empty() {
            return true;
        }

        if self.len == 0 || self.capacity == 0 {
            return false;
        }

        // Check backwards from the most recently inserted character
        let mut curr = if self.head == 0 {
            self.capacity - 1
        } else {
            self.head - 1
        };

        let mut count = 0;
        for ch in trigger.chars().rev() {
            count += 1;
            if count > self.len {
                return false;
            }
            if self.buffer[curr] != ch {
                return false;
            }
            curr = if curr == 0 {
                self.capacity - 1
            } else {
                curr - 1
            };
        }

        true
    }

    /// Returns the current buffer content as a String (for debugging).
    #[must_use]
    pub fn content(&self) -> String {
        if self.len == 0 {
            return String::new();
        }

        let start = if self.len < self.capacity {
            0
        } else {
            self.head
        };

        let mut res = String::with_capacity(self.len);
        for i in 0..self.len {
            let idx = (start + i) % self.capacity;
            res.push(self.buffer[idx]);
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_content() {
        let mut buf = KeyBuffer::new(5);
        buf.push('a');
        buf.push('b');
        buf.push('c');
        assert_eq!(buf.content(), "abc");
    }

    #[test]
    fn test_overflow() {
        let mut buf = KeyBuffer::new(3);
        buf.push('a');
        buf.push('b');
        buf.push('c');
        buf.push('d');
        assert_eq!(buf.content(), "bcd");
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn test_ends_with() {
        let mut buf = KeyBuffer::new(10);
        buf.push('h');
        buf.push('e');
        buf.push('l');
        buf.push('l');
        buf.push('o');
        assert!(buf.ends_with("llo"));
        assert!(buf.ends_with("hello"));
        assert!(!buf.ends_with("hell"));
        assert!(!buf.ends_with("hello!"));
        assert!(buf.ends_with(""));
    }

    #[test]
    fn test_pop() {
        let mut buf = KeyBuffer::new(3);
        buf.push('a');
        buf.push('b');
        assert_eq!(buf.pop(), Some('b'));
        assert_eq!(buf.content(), "a");
        buf.push('c');
        assert_eq!(buf.content(), "ac");
        assert_eq!(buf.pop(), Some('c'));
        assert_eq!(buf.pop(), Some('a'));
        assert_eq!(buf.pop(), None);
    }

    #[test]
    fn test_clear() {
        let mut buf = KeyBuffer::new(3);
        buf.push('a');
        buf.clear();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert_eq!(buf.content(), "");
    }
}
