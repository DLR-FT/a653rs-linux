use std::cell::UnsafeCell;
use std::fmt::{Debug, Formatter};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{mem, ptr};

/// An unsized bounded concurrent queue (Fifo) that makes use of atomics and
/// does not use pointers internally. This allows the queue to be
/// created inside a buffer of type `&[u8]` via [ConcurrentQueue::init_at].
/// The required buffer size can be requested in advance via
/// [ConcurrentQueue::size] by providing the size and maximum number of
/// entries. # Example
/// ```
/// # use a653rs_linux_core::queuing::queue::ConcurrentQueue;
/// // Create a ConcurrentQueue inside of a Vec<u8> buffer object
/// let required_size = ConcurrentQueue::size(1, 4);
/// let mut  buffer = vec![0u8; required_size];
/// ConcurrentQueue::init_at(&mut buffer, 1, 4);
/// let queue1 = unsafe { ConcurrentQueue::load_from(&buffer) };
/// let queue2 = unsafe { ConcurrentQueue::load_from(&buffer) };
///
/// // Let's push some values in the queue
/// assert!(queue1.push(&[1]).is_some());
/// assert!(queue2.push(&[2]).is_some());
///
/// // Now pop them using the Fifo method
/// assert_eq!(queue2.pop().unwrap()[0], 1);
/// assert_eq!(queue1.pop().unwrap()[0], 2);
///
/// // When the queue is empty, pop will return None
/// assert_eq!(queue1.pop(), None);
/// assert_eq!(queue2.pop(), None);
/// ```
#[repr(C)]
pub struct ConcurrentQueue {
    pub msg_size: usize,
    pub msg_capacity: usize,

    len: AtomicUsize,
    first: AtomicUsize,
    data: UnsafeCell<[u8]>,
}

unsafe impl Send for ConcurrentQueue {}
unsafe impl Sync for ConcurrentQueue {}

impl ptr_meta::Pointee for ConcurrentQueue {
    type Metadata = usize;
}

impl ConcurrentQueue {
    /// Calculates the required buffer size to fit a MessageQueue object
    /// with `capacity` maximum elements and a fixed size of `element_size`
    /// bytes per element.
    pub fn size(element_size: usize, capacity: usize) -> usize {
        let mut size = Self::fields_size() + element_size * capacity; // data

        // We need to include extra padding for calculating this structs size,
        // because of `#[repr(C)]` the compiler may add padding to this struct for
        // alignment purposes,
        let alignment = Self::align();
        let sub_alignment_mask = alignment - 1;
        if size & sub_alignment_mask > 0 {
            // If the size ended with non-aligned bytes, we add the necessary padding.
            size = (size & !sub_alignment_mask) + alignment;
        }

        size
    }

    /// Returns the size of bytes of this struct's fields
    fn fields_size() -> usize {
        mem::size_of::<usize>() // entry_size
                + mem::size_of::<usize>() // capacity
                + mem::size_of::<AtomicUsize>() // len
                + mem::size_of::<AtomicUsize>() // first
    }

    /// Returns this struct's alignment
    fn align() -> usize {
        // This structs maximum alignment is that of a usize (or AtomicUsize, which has
        // the same data layout)
        mem::align_of::<usize>()
    }

    /// Creates a new empty ConcurrentQueue in given buffer.
    /// Even though this function returns a reference to the newly created
    /// ConcurrentQueue, it should be dropped to release the mutable
    /// reference to the buffer.
    ///
    /// # Panics
    /// If the buffer size is not exactly the required size to fit this
    /// `ConcurrentQueue` object.
    pub fn init_at(buffer: &mut [u8], element_size: usize, capacity: usize) -> &Self {
        assert_eq!(buffer.len(), Self::size(element_size, capacity));

        // We cast the `buffer` reference to a `Self` pointer, which can then safely be
        // dereferenced
        let queue = unsafe { &mut *Self::buf_to_self_mut(buffer) };

        queue.msg_size = element_size;
        queue.msg_capacity = capacity;
        // Use `ptr::write` to prevent the compiler from trying to drop previous values.
        unsafe {
            ptr::write(&mut queue.len, AtomicUsize::new(0));
            ptr::write(&mut queue.first, AtomicUsize::new(0));
        }

        queue
    }

    /// Converts the given buffer pointer to a ConcurrentQueue pointer and
    /// handles shortening the wide-pointer metadata.
    fn buf_to_self(buffer: *const [u8]) -> *const Self {
        let (buf_ptr, mut buf_len): (*const (), usize) = ptr_meta::PtrExt::to_raw_parts(buffer);
        buf_len -= Self::fields_size();

        ptr_meta::from_raw_parts(buf_ptr, buf_len)
    }

    /// Converts the given mutable buffer pointer to a ConcurrentQueue
    /// pointer and handles shortening the wide-pointer metadata.
    fn buf_to_self_mut(buffer: *mut [u8]) -> *mut Self {
        let (buf_ptr, mut buf_len): (*mut (), usize) = ptr_meta::PtrExt::to_raw_parts(buffer);
        buf_len -= Self::fields_size();

        ptr_meta::from_raw_parts_mut(buf_ptr, buf_len)
    }

    /// Loads a `ConcurrentQueue` from the specified buffer.
    /// # Safety
    /// The buffer must contain exactly one valid ConcurrentQueue, which has
    /// to be initialized through [ConcurrentQueue::init_at]. Also
    /// mutating or reading raw values from the buffer may result in
    /// UB, because the ConcurrentQueue relies on internal safety mechanisms
    /// to prevent UB due to shared mutable state.
    pub unsafe fn load_from(buffer: &[u8]) -> &Self {
        let obj = &*Self::buf_to_self(buffer);

        // Perform some validity checks
        debug_assert!(obj.len.load(Ordering::SeqCst) <= obj.msg_capacity); // Check length
        debug_assert!(obj.first.load(Ordering::SeqCst) < obj.msg_capacity); // Check first idx

        // Also check if unsized data field is of correct size
        // Note: obj_data may be longer than `obj.msg_size * obj.msg_capacity` due to
        // alignment padding. To correct we call `Self::size`.
        let obj_data = obj.data.get().as_ref().unwrap();
        debug_assert_eq!(
            obj_data.len(),
            Self::size(obj.msg_size, obj.msg_capacity) - Self::fields_size()
        );

        obj
    }

    /// Calculates the physical starting index of an element inside of the
    /// data array.
    fn to_physical_idx(&self, first: usize, idx: usize) -> usize {
        (first + idx) % self.msg_capacity * self.msg_size
    }

    /// Gets an element from the queue at a specific index
    pub fn get(&self, idx: usize) -> Option<&[u8]> {
        assert!(idx < self.msg_capacity);

        let current_len = self.len.load(Ordering::SeqCst);
        if idx > current_len {
            return None;
        }

        let idx = self.to_physical_idx(self.first.load(Ordering::SeqCst), idx);

        let msg = &unsafe { self.data.get().as_mut().unwrap() }[idx..(idx + self.msg_size)];
        Some(msg)
    }

    /// Pushes an element to the back of the queue. If there was space, a
    /// mutable reference to the inserted element is returned.
    pub fn push(&self, data: &[u8]) -> Option<&mut [u8]> {
        assert_eq!(data.len(), self.msg_size);

        self.push_then(|entry| entry.copy_from_slice(data))
    }

    /// Pushes an uninitialized element and then calls a closure to set its
    /// memory in-place. If there was space, a mutable reference to
    /// the inserted element is returned.
    pub fn push_then<F: FnOnce(&'_ mut [u8])>(&self, set_element: F) -> Option<&mut [u8]> {
        let current_len = self.len.load(Ordering::SeqCst);
        if current_len == self.msg_capacity {
            return None;
        }

        let insert_idx = self.len.fetch_add(1, Ordering::SeqCst);

        let idx = self.to_physical_idx(self.first.load(Ordering::SeqCst), insert_idx);
        let element_slot =
            &mut unsafe { self.data.get().as_mut().unwrap() }[idx..(idx + self.msg_size)];

        set_element(element_slot);

        Some(element_slot)
    }

    /// Tries to pop an element from the front of the queue.
    pub fn pop(&self) -> Option<Box<[u8]>> {
        self.pop_then(|entry| Vec::from(entry).into_boxed_slice())
    }

    /// Calls a mapping closure on the first element that is about to be
    /// popped from the queue. Only the return value of the closure
    /// is returned by this function. If the popped element is
    /// needed as owned data, consider using [ConcurrentQueue::pop] instead.
    pub fn pop_then<F: FnOnce(&'_ [u8]) -> T, T>(&'_ self, map_element: F) -> Option<T> {
        // Decrement length
        self.len
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |len| len.checked_sub(1))
            .ok()?;

        let prev_first = self
            .first
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |x| {
                Some((x + 1) % self.msg_capacity)
            })
            .unwrap();

        let idx = self.to_physical_idx(prev_first, 0);

        let msg = &unsafe { &*self.data.get() }[idx..(idx + self.msg_size)];

        Some(map_element(msg))
    }

    pub fn peek_then<T, F: FnOnce(Option<&[u8]>) -> T>(&self, f: F) -> T {
        let len = self.len.load(Ordering::SeqCst);

        let msg = (len > 0).then(|| {
            let first = self.first.load(Ordering::SeqCst);
            let idx = self.to_physical_idx(first, 0);
            unsafe { &(&*self.data.get())[idx..(idx + self.msg_size)] }
        });

        f(msg)
    }

    /// Returns the current length of this queue
    pub fn len(&self) -> usize {
        self.len.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        self.len.store(0, Ordering::SeqCst);
    }
}

impl Debug for ConcurrentQueue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConcurrentQueue")
            .field("msg_size", &self.msg_size)
            .field("msg_capacity", &self.msg_capacity)
            .field("len", &self.len)
            .field("first", &self.first)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::ConcurrentQueue;

    #[test]
    fn single_queue() {
        const ELEMENT_SIZE: usize = 2;
        const CAPACITY: usize = 3;

        let mut buffer: Vec<u8> = vec![0u8; ConcurrentQueue::size(ELEMENT_SIZE, CAPACITY)];

        let _ = ConcurrentQueue::init_at(&mut buffer, ELEMENT_SIZE, CAPACITY);

        let queue = unsafe { ConcurrentQueue::load_from(&buffer) };

        assert_eq!(queue.pop(), None);
        assert_eq!(queue.pop(), None);

        assert_eq!(queue.push(&[0x4, 0x9]).unwrap(), &mut [0x4, 0x9]);
        assert_eq!(queue.push(&[0x2, 0xe]).unwrap(), &mut [0x2, 0xe]);
        assert_eq!(queue.push(&[0x1, 0x2]).unwrap(), &mut [0x1, 0x2]);

        // No more empty space
        assert_eq!(queue.push(&[0x3, 0x4]), None);

        assert_eq!(queue.pop(), Some(vec![0x4, 0x9].into_boxed_slice()));
        assert_eq!(queue.pop(), Some(vec![0x2, 0xe].into_boxed_slice()));
        assert_eq!(queue.pop(), Some(vec![0x1, 0x2].into_boxed_slice()));
        assert_eq!(queue.pop(), None);
    }

    #[test]
    fn two_queues() {
        const ELEMENT_SIZE: usize = 2;
        const CAPACITY: usize = 2;

        let mut buffer: Vec<u8> = vec![0u8; ConcurrentQueue::size(ELEMENT_SIZE, CAPACITY)];

        let _ = ConcurrentQueue::init_at(&mut buffer, ELEMENT_SIZE, CAPACITY);

        let (queue1, queue2) = unsafe {
            (
                ConcurrentQueue::load_from(&buffer),
                ConcurrentQueue::load_from(&buffer),
            )
        };
        assert_eq!(queue1.pop(), None);
        assert_eq!(queue2.pop(), None);

        assert_eq!(queue1.push(&[0x4, 0x9]).unwrap(), &mut [0x4, 0x9]);
        assert_eq!(queue2.push(&[0x2, 0xe]).unwrap(), &mut [0x2, 0xe]);

        // No more empty space
        assert_eq!(queue2.push(&[0x0, 0x0]), None);

        assert_eq!(queue2.pop(), Some(vec![0x4, 0x9].into_boxed_slice()));
        assert_eq!(queue2.pop(), Some(vec![0x2, 0xe].into_boxed_slice()));
        assert_eq!(queue2.pop(), None);
        assert_eq!(queue1.pop(), None);
    }

    #[test]
    /// Spawn one thousand threads with access to a single queue. Each thread
    /// pushes four elements on the queue. In the end validate that there is the
    /// correct amount of elements in the queue and that none of the elements
    /// are corrupted.
    fn one_thousand_queues() {
        use std::sync::Arc;
        use std::thread;

        const NUM_QUEUES: usize = 1000;

        const ELEMENT_SIZE: usize = 2;
        const CAPACITY: usize = 4 * NUM_QUEUES;

        let mut buffer: Vec<u8> = vec![0u8; ConcurrentQueue::size(ELEMENT_SIZE, CAPACITY)];
        let _ = ConcurrentQueue::init_at(&mut buffer, ELEMENT_SIZE, CAPACITY);

        // Put the buffer into an Arc to be able to access it from other threads
        let buffer: Arc<Vec<u8>> = Arc::new(buffer);

        // Spawn threads and create one queue per thread. Call push on the queue 4
        // times.
        let threads = (0..NUM_QUEUES).map(|_| {
            let cloned_buffer = buffer.clone();
            thread::spawn(move || {
                let queue = unsafe { ConcurrentQueue::load_from(&*cloned_buffer) };
                queue.push(&[0x1, 0x2]).unwrap();
                queue.push(&[0x3, 0x4]).unwrap();
                queue.push(&[0x5, 0x6]).unwrap();
                queue.push(&[0x7, 0x8]).unwrap();
            })
        });

        threads.for_each(|handle| handle.join().expect("that the thread has not panicked"));

        let queue = unsafe { ConcurrentQueue::load_from(&*buffer) };

        // There should be 4 elements per thread in the queue
        assert_eq!(queue.len(), 4 * NUM_QUEUES);

        // Check if there are any corrupted bytes
        while let Some(element) = queue.pop() {
            match &*element {
                &[0x1, 0x2] | &[0x3, 0x4] | &[0x5, 0x6] | &[0x7, 0x8] => {}
                invalid => panic!("Invalid bytes {invalid:?}"),
            }
        }
    }

    #[test]
    #[should_panic]
    fn buffer_too_small() {
        let mut buffer: Vec<u8> = vec![0u8; 2];
        let _ = ConcurrentQueue::init_at(&mut buffer, 1, 1);
    }

    #[test]
    #[should_panic]
    fn buffer_too_big() {
        let mut buffer: Vec<u8> = vec![0u8; 100];
        let _ = ConcurrentQueue::init_at(&mut buffer, 1, 1);
    }

    #[test]
    #[should_panic]
    fn element_too_big() {
        let mut buffer: Vec<u8> = vec![0u8; ConcurrentQueue::size(1, 1)];
        let queue = ConcurrentQueue::init_at(&mut buffer, 1, 1);

        queue.push(&[0x1, 0x1]);
    }

    #[test]
    #[should_panic]
    fn element_too_small() {
        let mut buffer: Vec<u8> = vec![0u8; ConcurrentQueue::size(2, 1)];
        let queue = ConcurrentQueue::init_at(&mut buffer, 2, 1);

        queue.push(&[0x1]);
    }

    #[test]
    #[should_panic]
    fn empty_element() {
        let mut buffer: Vec<u8> = vec![0u8; ConcurrentQueue::size(1, 1)];
        let queue = ConcurrentQueue::init_at(&mut buffer, 1, 1);

        queue.push(&[]);
    }
}
