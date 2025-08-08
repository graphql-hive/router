use bytes::{BufMut, BytesMut};

use crate::json_writer::{write_and_escape_string, write_f64, write_i64, write_u64};
use crate::utils::consts::{
    CLOSE_BRACE, CLOSE_BRACKET, COLON, COMMA, FALSE, NULL, OPEN_BRACE, OPEN_BRACKET, QUOTE, TRUE,
};

pub struct ResponseWriter<'a> {
    buffer: &'a mut BytesMut,
    is_first_field: bool,
}

impl<'a> ResponseWriter<'a> {
    pub fn new(buffer: &'a mut BytesMut) -> Self {
        Self {
            buffer,
            is_first_field: true,
        }
    }

    pub fn start_object(&mut self) {
        self.buffer.put_slice(OPEN_BRACE);
        self.is_first_field = true;
    }

    pub fn end_object(&mut self) {
        self.buffer.put_slice(CLOSE_BRACE);
    }

    pub fn start_array(&mut self) {
        self.buffer.put_slice(OPEN_BRACKET);
        self.is_first_field = true;
    }

    pub fn end_array(&mut self) {
        self.buffer.put_slice(CLOSE_BRACKET);
    }

    fn start_field(&mut self) {
        if self.is_first_field {
            self.is_first_field = false;
        } else {
            self.write_separator();
        }
    }

    pub fn write_key(&mut self, key: &str) {
        self.start_field();
        self.buffer.put_slice(QUOTE);
        self.buffer.put_slice(key.as_bytes());
        self.buffer.put_slice(QUOTE);
        self.buffer.put_slice(COLON);
    }

    pub fn write_null(&mut self) {
        self.buffer.put_slice(NULL);
    }

    pub fn write_bool(&mut self, value: bool) {
        if value {
            self.buffer.put_slice(TRUE);
        } else {
            self.buffer.put_slice(FALSE);
        }
    }

    pub fn write_u64(&mut self, value: u64) {
        write_u64(self.buffer, value);
    }

    pub fn write_i64(&mut self, value: i64) {
        write_i64(self.buffer, value);
    }

    pub fn write_f64(&mut self, value: f64) {
        write_f64(self.buffer, value);
    }

    pub fn write_string(&mut self, value: &str) {
        write_and_escape_string(self.buffer, value);
    }

    pub fn write_separator(&mut self) {
        self.buffer.put_slice(COMMA);
    }

    pub fn write_raw_slice(&mut self, slice: &[u8]) {
        self.buffer.put_slice(slice);
    }
}
