use oxedyne_fe2o3_core::prelude::*;

use std::{
    sync::Arc,
};

/// The immutable, thread-safe stack from [Learning Rust with Entirely Too Many Lists](https://rust-unofficial.github.io/too-many-lists/third-final.html).
#[derive(Clone, Debug, PartialEq)]
pub struct Stack<T> {
    head: Link<T>,
}

type Link<T> = Option<Arc<Node<T>>>;

#[derive(Clone, Debug, PartialEq)]
struct Node<T> {
    data: T,
    next: Link<T>,
}

impl<T> Stack<T> {
    pub fn new() -> Self {
        Stack {
            head: None,
        }
    }

    /// Append to the head of the stack.
    // stack1 -> A ---+             stack1 = A -> B -> C -> D
    //                |            
    //                v
    // stack2 ------> B -> C -> D   stack2 = tail(stack1) = B -> C -> D
    //                ^
    //                |
    // stack3 -> X ---+             stack3 = push(stack2, X) = X -> B -> C -> D
    pub fn push(&self, data: T) -> Self {
        Stack {
            head: Some(Arc::new(Node {
                data: data,
                next: self.head.clone(),
            }))
        }
    }
    
    /// Snip off head and return the tail.
    pub fn tail(&self) -> Self {
        Stack {
            //head: self.head.as_ref().and_then(|node| node.next.clone())
            head: match self.head.as_ref() {
                Some(node) => node.next.clone(),
                None => None,
            },
        }
    }

    pub fn head(&self) -> Option<&T> {
        //self.head.as_ref().map(|node| &node.data)
        match self.head.as_ref() {
            Some(node) => Some(&node.data),
            None => None,
        }
    }

    pub fn iter(&self) -> StackIter<'_, T> {
        StackIter {
            //next: self.head.as_ref().map(|node| &**node)
            next: match self.head.as_ref() {
                Some(node) => Some(&**node),
                None => None,
            }
        }
    }

    //pub fn peek(&self) -> Option<&T> {
    //    self.head.as_ref().map(|node| {
    //        &node.data
    //    })
    //}
}

impl<T> Drop for Stack<T> {
    fn drop(&mut self) {
        let mut head = self.head.take();
        while let Some(node) = head {
            if let Ok(mut node) = Arc::try_unwrap(node) {
                head = node.next.take();
            } else {
                break;
            }
        }
    }
}

pub struct StackIter<'a, T> {
    next: Option<&'a Node<T>>,
}

impl<'a, T> Iterator for StackIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        //self.next.map(|node| {
        //    self.next = node.next.as_ref().map(|node| &**node);
        //    &node.data
        //})
        match self.next {
            Some(node) => {
                self.next = match node.next.as_ref() {
                    Some(node) => Some(&**node),
                    None => None,
                };
                Some(&node.data)
            },
            None => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::Stack;

    #[test]
    fn test_stack_basics() {
        let list = Stack::new();
        assert_eq!(list.head(), None);

        let list = list.push(1).push(2).push(3);
        assert_eq!(list.head(), Some(&3));

        let list = list.tail();
        assert_eq!(list.head(), Some(&2));

        let list = list.tail();
        assert_eq!(list.head(), Some(&1));

        let list = list.tail();
        assert_eq!(list.head(), None);

        // Make sure empty tail works
        let list = list.tail();
        assert_eq!(list.head(), None);
    }

    #[test]
    fn test_stack_iter() {
        let list = Stack::new().push(1).push(2).push(3);

        let mut iter = list.iter();
        assert_eq!(iter.next(), Some(&3));
        assert_eq!(iter.next(), Some(&2));
        assert_eq!(iter.next(), Some(&1));
    }

    #[test]
    fn test_stack_tuple_iter() {
        let mut list = Stack::<(u8, u8)>::new();
        list = list.push((42, 1)).push((6, 2)).push((17, 3));


        let mut iter = list.iter();
        assert_eq!(iter.next(), Some(&(17, 3)));
        assert_eq!(iter.next(), Some(&(6, 2)));
        assert_eq!(iter.next(), Some(&(42, 1)));
        assert_eq!(iter.next(), None);
    }
}
