// Copyright 2024 Ulvetanna Inc.
use bytes::{Buf, BufMut};

#[derive(Clone, thiserror::Error, Debug)]
pub enum Error {
	#[error("Write buffer is full")]
	WriteBufferFull,
	#[error("Not enough data in read buffer to deserialize")]
	NotEnoughBytes,
}

pub trait SerializeBytes {
	fn serialize(&self, write_buf: impl BufMut) -> Result<(), Error>;
}

pub trait DeserializeBytes {
	fn deserialize(read_buf: impl Buf) -> Result<Self, Error>
	where
		Self: Sized;
}
