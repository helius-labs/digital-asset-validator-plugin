use crate::{
    error::CMTError,
    state::{ChangeLog, Node, Path, EMPTY},
    utils::{empty_node, fill_in_proof, recompute},
};
use bytemuck::{Pod, Zeroable};
pub(crate) use log_compute;
pub(crate) use solana_logging;

use solana_program::keccak::hashv;

#[cfg(feature = "sol-log")]
use solana_program::{log::sol_log_compute_units, msg};

/// Tracks updates to off-chain Merkle tree
///
/// Allows for concurrent writes to same merkle tree so long as proof
/// was generated for a that has had at most MAX_SIZE updates since the tx was submitted
#[derive(Copy, Clone)]
pub struct MerkleRoll<const MAX_DEPTH: usize, const MAX_BUFFER_SIZE: usize> {
    pub sequence_number: u128,
    /// Index of most recent root & changes
    pub active_index: u64,
    /// Number of active changes we are tracking
    pub buffer_size: u64,
    /// Proof for respective root
    pub change_logs: [ChangeLog<MAX_DEPTH>; MAX_BUFFER_SIZE],
    pub rightmost_proof: Path<MAX_DEPTH>,
}

unsafe impl<const MAX_DEPTH: usize, const MAX_BUFFER_SIZE: usize> Zeroable
    for MerkleRoll<MAX_DEPTH, MAX_BUFFER_SIZE>
{
}
unsafe impl<const MAX_DEPTH: usize, const MAX_BUFFER_SIZE: usize> Pod
    for MerkleRoll<MAX_DEPTH, MAX_BUFFER_SIZE>
{
}

impl<const MAX_DEPTH: usize, const MAX_BUFFER_SIZE: usize> MerkleRoll<MAX_DEPTH, MAX_BUFFER_SIZE> {
    pub fn new() -> Self {
        Self {
            sequence_number: 0,
            active_index: 0,
            buffer_size: 0,
            change_logs: [ChangeLog::<MAX_DEPTH>::default(); MAX_BUFFER_SIZE],
            rightmost_proof: Path::<MAX_DEPTH>::default(),
        }
    }

    pub fn initialize(&mut self) -> Result<Node, CMTError> {
        let mut rightmost_proof = Path::default();
        for (i, node) in rightmost_proof.proof.iter_mut().enumerate() {
            *node = empty_node(i as u32);
        }
        let mut path = [Node::default(); MAX_DEPTH];
        for (i, node) in path.iter_mut().enumerate() {
            *node = empty_node(i as u32);
        }
        self.change_logs[0].root = empty_node(MAX_DEPTH as u32);
        self.change_logs[0].path = path;
        self.sequence_number = 0;
        self.active_index = 0;
        self.buffer_size = 1;
        self.rightmost_proof = rightmost_proof;
        Ok(self.change_logs[0].root)
    }

    pub fn initialize_with_root(
        &mut self,
        root: Node,
        rightmost_leaf: Node,
        proof_vec: Vec<Node>,
        index: u32,
    ) -> Result<Node, CMTError> {
        let mut proof: [Node; MAX_DEPTH] = [Node::default(); MAX_DEPTH];
        proof.copy_from_slice(&proof_vec[..]);
        let rightmost_proof = Path {
            proof,
            index: index + 1,
            leaf: rightmost_leaf,
            _padding: 0,
        };
        self.change_logs[0].root = root;
        self.sequence_number = 1;
        self.active_index = 0;
        self.buffer_size = 1;
        self.rightmost_proof = rightmost_proof;
        assert_eq!(root, recompute(rightmost_leaf, &proof, index,));
        Ok(root)
    }

    pub fn get_change_log(&self) -> Box<ChangeLog<MAX_DEPTH>> {
        Box::new(self.change_logs[self.active_index as usize])
    }

    /// Only used to initialize right most path for a completely empty tree
    #[inline(always)]
    fn initialize_tree(
        &mut self,
        leaf: Node,
        mut proof: [Node; MAX_DEPTH],
    ) -> Result<Node, CMTError> {
        let old_root = recompute(EMPTY, &proof, 0);
        if old_root == empty_node(MAX_DEPTH as u32) {
            self.update_and_apply_proof(EMPTY, leaf, &mut proof, 0, 0, false, false)
        } else {
            return Err(CMTError::TreeAlreadyInitialized);
        }
    }

    /// Basic operation that always succeeds
    pub fn append(&mut self, mut node: Node) -> Result<Node, CMTError> {
        if node == EMPTY {
            return Err(CMTError::CannotAppendEmptyNode);
        }
        if self.rightmost_proof.index >= 1 << MAX_DEPTH {
            return Err(CMTError::TreeFull);
        }
        if self.rightmost_proof.index == 0 {
            return self.initialize_tree(node, self.rightmost_proof.proof);
        }
        let leaf = node.clone();
        let intersection = self.rightmost_proof.index.trailing_zeros() as usize;
        let mut change_list = [EMPTY; MAX_DEPTH];
        let mut intersection_node = self.rightmost_proof.leaf;

        // Compute proof to the appended node from empty nodes
        for i in 0..intersection {
            change_list[i] = node;
            let hash = hashv(&[node.as_ref(), empty_node(i as u32).as_ref()]);
            node.copy_from_slice(hash.as_ref());
            let rightmost_hash = if ((self.rightmost_proof.index - 1) >> i) & 1 == 1 {
                hashv(&[
                    self.rightmost_proof.proof[i].as_ref(),
                    intersection_node.as_ref(),
                ])
            } else {
                hashv(&[
                    intersection_node.as_ref(),
                    self.rightmost_proof.proof[i].as_ref(),
                ])
            };
            intersection_node.copy_from_slice(rightmost_hash.as_ref());
            self.rightmost_proof.proof[i] = empty_node(i as u32);
        }

        // Compute the where the new node intersects the main tree
        change_list[intersection] = node;
        let hash = hashv(&[intersection_node.as_ref(), node.as_ref()]);
        node.copy_from_slice(hash.as_ref());
        self.rightmost_proof.proof[intersection] = intersection_node;

        // Update the change list path up to the root
        for i in intersection + 1..MAX_DEPTH {
            change_list[i] = node;
            let hash = if (self.rightmost_proof.index >> i) & 1 == 1 {
                hashv(&[self.rightmost_proof.proof[i].as_ref(), node.as_ref()])
            } else {
                hashv(&[node.as_ref(), self.rightmost_proof.proof[i].as_ref()])
            };
            node.copy_from_slice(hash.as_ref());
        }

        self.increment_active_index();
        self.change_logs[self.active_index as usize] = ChangeLog::<MAX_DEPTH> {
            root: node,
            path: change_list,
            index: self.rightmost_proof.index,
            _padding: 0,
        };
        self.rightmost_proof.index = self.rightmost_proof.index + 1;
        self.rightmost_proof.leaf = leaf;
        self.sequence_number = self.sequence_number.saturating_add(1);
        Ok(node)
    }

    /// Convenience function for `set_leaf`
    /// On write conflict:
    /// Will append
    pub fn fill_empty_or_append(
        &mut self,
        current_root: Node,
        leaf: Node,
        proof_vec: Vec<Node>,
        index: u32,
    ) -> Result<Node, CMTError> {
        let mut proof: [Node; MAX_DEPTH] = [Node::default(); MAX_DEPTH];
        fill_in_proof::<MAX_DEPTH>(proof_vec, &mut proof);
        log_compute!();
        let root = self.find_and_update_leaf(current_root, EMPTY, leaf, &mut proof, index, true);
        log_compute!();
        root
    }

    /// On write conflict:
    /// Will fail by returning None
    pub fn set_leaf(
        &mut self,
        current_root: Node,
        previous_leaf: Node,
        new_leaf: Node,
        proof_vec: Vec<Node>,
        index: u32,
    ) -> Result<Node, CMTError> {
        if index > self.rightmost_proof.index {
            return Err(CMTError::LeafIndexOutOfBounds);
        } else {
            let mut proof: [Node; MAX_DEPTH] = [Node::default(); MAX_DEPTH];
            fill_in_proof::<MAX_DEPTH>(proof_vec, &mut proof);

            log_compute!();
            let root = self.find_and_update_leaf(
                current_root,
                previous_leaf,
                new_leaf,
                &mut proof,
                index,
                false,
            );
            log_compute!();
            root
        }
    }

    /// Internal function used to set leaf value & record changelog
    fn find_and_update_leaf(
        &mut self,
        current_root: Node,
        leaf: Node,
        new_leaf: Node,
        proof: &mut [Node; MAX_DEPTH],
        index: u32,
        append_on_conflict: bool,
    ) -> Result<Node, CMTError> {
        solana_logging!("Active Index: {}", self.active_index);
        solana_logging!("Rightmost Index: {}", self.rightmost_proof.index);
        solana_logging!("Buffer Size: {}", self.buffer_size);
        solana_logging!("Leaf Index: {}", index);
        let mask: usize = MAX_BUFFER_SIZE - 1;

        for i in 0..self.buffer_size {
            let j = self.active_index.wrapping_sub(i) & mask as u64;
            if self.change_logs[j as usize].root != current_root {
                continue;
            }
            return self.update_and_apply_proof(
                leaf,
                new_leaf,
                proof,
                index,
                j,
                append_on_conflict,
                false,
            );
        }
        solana_logging!("Failed to find root, attempting to replay change log");
        // Optimistic search
        self.update_and_apply_proof(
            leaf,
            new_leaf,
            proof,
            index,
            self.active_index.wrapping_sub(self.buffer_size) & mask as u64,
            append_on_conflict,
            true,
        )
    }

    /// Fast-forwards submitted proof to be valid for the root at `self.current_index`
    ///
    /// Updates proof & updates root & stores
    ///
    /// Takes in `j`, which is the root index that this proof was last valid for
    fn update_and_apply_proof(
        &mut self,
        leaf: Node,
        new_leaf: Node,
        proof: &mut [Node; MAX_DEPTH],
        index: u32,
        mut j: u64,
        append_on_conflict: bool,
        use_full_buffer: bool,
    ) -> Result<Node, CMTError> {
        let mut updated_leaf = leaf;
        solana_logging!("Fast-forwarding proof, starting index {}", j);
        let mask: usize = MAX_BUFFER_SIZE - 1;
        let padding: usize = 32 - MAX_DEPTH;
        log_compute!();
        // Modifies proof by iterating through the change log
        loop {
            // If use_full_buffer is false, this loop will terminate if the initial value of j is the active index
            if !use_full_buffer && j == self.active_index {
                break;
            }
            j += 1;
            j &= mask as u64;
            if index != self.change_logs[j as usize].index {
                let common_path_len = ((index ^ self.change_logs[j as usize].index) << padding)
                    .leading_zeros() as usize;
                let critbit_index = (MAX_DEPTH - 1) - common_path_len;
                proof[critbit_index] = self.change_logs[j as usize].path[critbit_index];
            } else {
                updated_leaf = self.change_logs[j as usize].get_leaf();
            }
            // If use_full_buffer is true, this loop will do 1 full pass of the change logs
            if use_full_buffer && j == self.active_index {
                break;
            }
        }
        log_compute!();
        let valid_root = recompute(updated_leaf, proof, index) == self.get_change_log().root;
        if updated_leaf != leaf || index > self.rightmost_proof.index {
            // If the supplied root was not found in the queue, the instruction should fail if the leaf index changes
            if !use_full_buffer && valid_root && leaf == EMPTY && append_on_conflict {
                return self.append(new_leaf);
            } else {
                return Err(CMTError::LeafAlreadyUpdated);
            }
        }
        if valid_root {
            self.increment_active_index();
            self.sequence_number = self.sequence_number.saturating_add(1);
            Ok(self.apply_changes(new_leaf, proof, index))
        } else {
            return Err(CMTError::InvalidProof);
        }
    }

    fn increment_active_index(&mut self) {
        let mask: usize = MAX_BUFFER_SIZE - 1;

        self.active_index += 1;
        self.active_index &= mask as u64;
        if self.buffer_size < MAX_BUFFER_SIZE as u64 {
            self.buffer_size += 1;
        }
    }

    /// Creates a new root from a proof that is valid for the root at `self.active_index`
    fn apply_changes(&mut self, start: Node, proof: &[Node], index: u32) -> Node {
        let padding: usize = 32 - MAX_DEPTH;
        let change_log = &mut self.change_logs[self.active_index as usize];
        change_log.index = index;

        // Also updates change_log's current root
        let root = change_log.recompute_path(start, proof);

        // Update rightmost path if possible
        if self.rightmost_proof.index < (1 << MAX_DEPTH) {
            if index < self.rightmost_proof.index as u32 {
                if index != self.rightmost_proof.index - 1 {
                    let common_path_len = ((index ^ (self.rightmost_proof.index - 1) as u32)
                        << padding)
                        .leading_zeros() as usize;
                    let critbit_index = (MAX_DEPTH - 1) - common_path_len;
                    self.rightmost_proof.proof[critbit_index] = change_log.path[critbit_index];
                }
            } else {
                assert!(index == self.rightmost_proof.index);
                solana_logging!("Appending rightmost leaf");
                self.rightmost_proof.proof.copy_from_slice(&proof);
                self.rightmost_proof.index = index + 1;
                self.rightmost_proof.leaf = change_log.get_leaf();
            }
        }
        root
    }
}