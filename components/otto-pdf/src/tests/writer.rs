//! A minimal PDF writer for test documents: numbered objects, a cross
//! reference table and a trailer.

// Rust guideline compliant 2026-02-21

/// Collects objects and writes them out as a PDF file.
pub struct Writer {
    objects: Vec<Option<String>>,
}

impl Writer {
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    /// A number for an object written later with [`set`](Self::set).
    pub fn reserve(&mut self) -> usize {
        self.objects.push(None);
        self.objects.len()
    }

    pub fn set(&mut self, id: usize, body: impl Into<String>) {
        self.objects[id - 1] = Some(body.into());
    }

    pub fn add(&mut self, body: impl Into<String>) -> usize {
        let id = self.reserve();
        self.set(id, body);
        id
    }

    /// A stream object: `dict` holds the entries besides `/Length`.
    pub fn add_stream(&mut self, dict: &str, data: &str) -> usize {
        self.add(format!(
            "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
            data.len() + 1
        ))
    }

    /// The file, with object `root` as its catalog.
    pub fn finish(self, root: usize) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            let body = body.as_deref().expect("every reserved object is set");
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {root} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}
