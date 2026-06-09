use crate::prelude::*;

/**************************************************************/

#[derive(Debug, Clone)]
pub enum List<T: Clone> {
    Cons(T, Arc<List<T>>),
    Nil,
}

impl<T: Clone> List<T> {
    pub fn new() -> Self {
        List::Nil
    }

    pub fn cons(self, t: T) -> Self {
        List::Cons(t, Arc::new(self))
    }

    pub fn head(&self) -> Option<&T> {
        match self {
            List::Cons(t, _) => Some(t),
            List::Nil => None,
        }
    }

    pub fn tail(self) -> Option<Self> {
        match self {
            List::Cons(_, l) => Some((*l).clone()),
            List::Nil => None,
        }
    }
}

impl<T: Clone> Default for List<T> {
    fn default() -> Self {
        Self::new()
    }
}

/**************************************************************/

#[derive(Debug, Clone)]
pub struct Zipper<T: Clone> {
    pub list: List<T>,
    pub anti: List<T>,
}

impl<T: Clone> Zipper<T> {
    pub fn new() -> Self {
        Zipper {
            list: List::new(),
            anti: List::new(),
        }
    }

    pub fn sing(t: T) -> Self {
        Zipper::new().cons(t)
    }

    pub fn cons(self, t: T) -> Self {
        Zipper {
            list: self.list.cons(t),
            anti: List::default(),
        }
    }

    pub fn head(&self) -> Option<&T> {
        self.list.head()
    }

    pub fn head_mut(&mut self) -> Option<&mut T> {
        match &mut self.list {
            List::Cons(t, _) => Some(t),
            List::Nil => None,
        }
    }

    pub fn tail(self) -> Option<Self> {
        match self.list {
            List::Cons(head, tail) => Some(Zipper {
                list: (*tail).clone(),
                anti: self.anti.cons(head),
            }),
            List::Nil => None,
        }
    }

    pub fn anti_head(&self) -> Option<&T> {
        self.anti.head()
    }

    pub fn back(self) -> Option<Self> {
        match self.anti {
            List::Cons(head, tail) => Some(Zipper {
                list: self.list.cons(head),
                anti: (*tail).clone(),
            }),
            List::Nil => None,
        }
    }
}

impl<T: Clone> Default for Zipper<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl Zipper<Box<str>> {
    pub fn one(s: &str) -> Self {
        Zipper::new().cons(s.into())
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zipper() {
        let mut z = Zipper::new();
        z = z.cons(1);
        assert_eq!(z.head().unwrap(), &1);
        z = z.cons(2);
        assert_eq!(z.head().unwrap(), &2);
        z = z.cons(3);
        assert_eq!(z.head().unwrap(), &3);
        z = z.tail().unwrap();
        assert_eq!(z.head().unwrap(), &2);
        z = z.back().unwrap();
        assert_eq!(z.head().unwrap(), &3);
    }
}
