use std::{borrow::Cow, fmt, io::Write, iter, mem};

use html5ever::{
    serialize::{self, Serializer},
    tendril::StrTendril,
    Attribute, QualName,
};

pub mod selector;
mod traverser;

use selector::{ContextualSelector, Selector};
pub use traverser::*;

#[derive(Clone)]
pub struct HtmlPathElement<'a, Handle> {
    pub handle: Handle,
    pub name: html5ever::QualName,
    pub attrs: Cow<'a, [Attribute]>,
}

impl<Handle> HtmlPathElement<'_, Handle> {
    pub fn attr(&self, name: QualName) -> Option<&StrTendril> {
        self.attrs
            .iter()
            .find_map(|a| (a.name == name).then_some(&a.value))
    }

    pub fn classes(&self) -> iter::Flatten<std::option::IntoIter<std::str::SplitWhitespace<'_>>> {
        use html5ever::*;
        const CLASS: QualName = QualName {
            prefix: None,
            ns: ns!(),
            local: local_name!("class"),
        };
        self.attr(CLASS)
            .map(|value| value.split_whitespace())
            .into_iter()
            .flatten()
    }
}

impl<Handle: fmt::Display> fmt::Display for HtmlPathElement<'_, Handle> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{} <{}", self.handle, &*self.name.local)?;
        for Attribute { name, value } in self.attrs.iter() {
            write!(f, " {}=\"{}\"", &*name.local, value)?;
        }
        write!(f, ">")
    }
}

pub type HtmlContext<'a, Handle> = &'a [HtmlPathElement<'a, Handle>];

pub trait HtmlSink<Handle>: Sized
where
    Handle: Eq + Copy,
{
    type Output;

    fn append_doctype_to_document(
        &mut self,
        name: &html5ever::tendril::StrTendril,
        public_id: &html5ever::tendril::StrTendril,
        system_id: &html5ever::tendril::StrTendril,
    );

    fn append_element(
        &mut self,
        context: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    );

    fn append_text(&mut self, context: HtmlContext<Handle>, text: &str);

    fn append_comment(&mut self, context: HtmlContext<Handle>, text: &str);

    fn reset(&mut self) -> Self::Output;

    fn finish(mut self) -> Self::Output {
        self.reset()
    }
}

struct OpenElement<Handle> {
    handle: Handle,
    name: html5ever::QualName,
}

pub struct HtmlSerializer<Wr: Write, Handle> {
    inner: html5ever::serialize::HtmlSerializer<Wr>,
    open_element_path: Vec<OpenElement<Handle>>,
}

impl<Wr: Write, Handle: Eq + fmt::Display> HtmlSerializer<Wr, Handle> {
    fn pop_to_path(&mut self, context: HtmlContext<'_, Handle>) {
        assert!(context
            .iter()
            .zip(&self.open_element_path)
            .all(|(a, b)| a.handle == b.handle));
        if context.len() > self.open_element_path.len() {
            panic!(
                "Non-appended elements in context : {}",
                context[self.open_element_path.len()..]
                    .iter()
                    .map(ToString::to_string)
                    .collect::<String>()
            );
        }
        while context.len() < self.open_element_path.len() {
            let closed = self.open_element_path.pop().unwrap();
            self.inner.end_elem(closed.name).unwrap();
        }
    }

    pub fn new(writer: Wr, opts: serialize::SerializeOpts) -> Self {
        Self {
            inner: html5ever::serialize::HtmlSerializer::new(writer, opts),
            open_element_path: vec![],
        }
    }
}

impl<Wr: Write, Handle: Eq + Copy + fmt::Display> HtmlSink<Handle>
    for &mut HtmlSerializer<Wr, Handle>
{
    type Output = ();

    fn append_element(
        &mut self,
        context: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    ) {
        self.pop_to_path(context);

        self.inner
            .start_elem(
                element.name.clone(),
                element.attrs.iter().map(|att| (&att.name, &*att.value)),
            )
            .unwrap();
        self.open_element_path.push(OpenElement {
            handle: element.handle,
            name: element.name.clone(),
        });
    }

    fn append_text(&mut self, context: HtmlContext<Handle>, text: &str) {
        self.pop_to_path(context);

        self.inner.write_text(text).unwrap();
    }

    fn append_comment(&mut self, context: HtmlContext<Handle>, text: &str) {
        self.pop_to_path(context);

        self.inner.write_comment(text).unwrap();
    }

    fn reset(&mut self) -> Self::Output {
        self.pop_to_path(&[])
    }

    fn append_doctype_to_document(
        &mut self,
        name: &html5ever::tendril::StrTendril,
        _public_id: &html5ever::tendril::StrTendril,
        _system_id: &html5ever::tendril::StrTendril,
    ) {
        self.inner.write_doctype(name).unwrap()
    }
}

pub struct ElementRemover<Handle: Eq + Copy, S: HtmlSink<Handle>, M: Selector> {
    inner: S,
    matcher: M,
    skip_handle: Option<Handle>,
}

impl<Handle: Eq + Copy, S: HtmlSink<Handle>, M: Selector> ElementRemover<Handle, S, M> {
    pub fn wrap(sink: S, matcher: M) -> Self {
        Self {
            inner: sink,
            matcher,
            skip_handle: None,
        }
    }
}

impl<Handle: Eq + Copy, S: HtmlSink<Handle>, M: Selector> HtmlSink<Handle>
    for ElementRemover<Handle, S, M>
{
    type Output = S::Output;

    fn append_doctype_to_document(
        &mut self,
        name: &html5ever::tendril::StrTendril,
        public_id: &html5ever::tendril::StrTendril,
        system_id: &html5ever::tendril::StrTendril,
    ) {
        self.inner
            .append_doctype_to_document(name, public_id, system_id)
    }

    fn append_element(
        &mut self,
        context: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    ) {
        if let Some(skip_handle) = self.skip_handle {
            if context.iter().any(|elem| elem.handle == skip_handle) {
                return;
            } else {
                self.skip_handle = None
            }
        }
        let skip = self.matcher.context_match(context, element);
        if skip {
            self.skip_handle = Some(element.handle);
            return;
        }
        self.inner.append_element(context, element)
    }

    fn append_text(&mut self, context: HtmlContext<Handle>, text: &str) {
        if let Some(skip_handle) = self.skip_handle {
            if context.iter().any(|elem| elem.handle == skip_handle) {
                return;
            } else {
                self.skip_handle = None
            }
        }
        self.inner.append_text(context, text)
    }

    fn append_comment(&mut self, context: HtmlContext<Handle>, text: &str) {
        if let Some(skip_handle) = self.skip_handle {
            if context.iter().any(|elem| elem.handle == skip_handle) {
                return;
            } else {
                self.skip_handle = None
            }
        }
        self.inner.append_comment(context, text)
    }

    fn reset(&mut self) -> Self::Output {
        self.skip_handle = None;
        self.inner.reset()
    }
}

pub struct RootFilter<Handle: Eq + Copy, S: HtmlSink<Handle>, M: ContextualSelector, O = ()> {
    inner: S,
    matcher: M,
    select_handle: Option<Handle>,
    output: O,
}

impl<Handle: Eq + Copy, S: HtmlSink<Handle>, M: ContextualSelector, O: Default>
    RootFilter<Handle, S, M, O>
{
    pub fn wrap(inner: S, matcher: M) -> Self {
        Self {
            inner,
            matcher,
            select_handle: None,
            output: O::default(),
        }
    }
}

impl<Handle, S, M: ContextualSelector, O> HtmlSink<Handle> for RootFilter<Handle, S, M, O>
where
    Handle: Eq + Copy,
    S: HtmlSink<Handle>,
    O: Extend<S::Output> + Default,
{
    type Output = O;

    fn append_doctype_to_document(
        &mut self,
        _name: &html5ever::tendril::StrTendril,
        _public_id: &html5ever::tendril::StrTendril,
        _system_id: &html5ever::tendril::StrTendril,
    ) {
    }

    fn append_element(
        &mut self,
        context: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    ) {
        if let Some(select_handle) = self.select_handle {
            if let Some(select_index) = context
                .iter()
                .enumerate()
                .find_map(|(index, elem)| (elem.handle == select_handle).then_some(index))
            {
                // select continues
                self.inner.append_element(&context[select_index..], element);
                return;
            } else {
                // select ends
                self.select_handle = None;
                self.output.extend(iter::once(self.inner.reset()));
            }
        }
        let select = self.matcher.context_match(context, element);
        if select {
            // select starts
            let select_handle = element.handle;
            self.inner.append_element(&[], element);
            self.select_handle = Some(select_handle);
        }
    }

    fn append_text(&mut self, context: HtmlContext<Handle>, text: &str) {
        if let Some(select_handle) = self.select_handle {
            if let Some(select_index) = context
                .iter()
                .enumerate()
                .find_map(|(index, elem)| (elem.handle == select_handle).then_some(index))
            {
                // select continues
                self.inner.append_text(&context[select_index..], text)
            } else {
                // select ends
                self.select_handle = None;
                self.output.extend(iter::once(self.inner.reset()));
            }
        }
    }

    fn append_comment(&mut self, context: HtmlContext<Handle>, text: &str) {
        if let Some(select_handle) = self.select_handle {
            if let Some(select_index) = context
                .iter()
                .enumerate()
                .find_map(|(index, elem)| (elem.handle == select_handle).then_some(index))
            {
                // select continues
                self.inner.append_comment(&context[select_index..], text)
            } else {
                // select ends
                self.select_handle = None;
                self.output.extend(iter::once(self.inner.reset()));
            }
        }
    }

    fn reset(&mut self) -> Self::Output {
        if self.select_handle.take().is_some() {
            self.output.extend(iter::once(self.inner.reset()));
            self.select_handle = None
        }
        mem::take(&mut self.output)
    }
}

pub struct ElementSkipper<S, M> {
    inner: S,
    matcher: M,
}

impl<S, M: Selector> ElementSkipper<S, M> {
    pub fn wrap(inner: S, matcher: M) -> Self {
        Self { inner, matcher }
    }
}

impl<Handle, S, M: Selector> HtmlSink<Handle> for ElementSkipper<S, M>
where
    Handle: Eq + Copy,
    S: HtmlSink<Handle>,
{
    type Output = S::Output;

    fn append_doctype_to_document(
        &mut self,
        _name: &html5ever::tendril::StrTendril,
        _public_id: &html5ever::tendril::StrTendril,
        _system_id: &html5ever::tendril::StrTendril,
    ) {
    }

    fn append_element(
        &mut self,
        context: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    ) {
        if self.matcher.context_match(context, element) {
            return;
        }
        // TODO optimise when not hitting
        let filtered_path = context
            .iter()
            .filter(|element| !self.matcher.is_match(element))
            .cloned()
            .collect::<Vec<_>>();
        self.inner.append_element(filtered_path.as_slice(), element);
    }

    fn append_text(&mut self, context: HtmlContext<Handle>, text: &str) {
        // TODO optimise when not hitting
        let filtered_path = context
            .iter()
            .filter(|element| !self.matcher.is_match(element))
            .cloned()
            .collect::<Vec<_>>();
        self.inner.append_text(filtered_path.as_slice(), text);
    }

    fn append_comment(&mut self, context: HtmlContext<Handle>, text: &str) {
        // TODO optimise when not hitting
        let filtered_path = context
            .iter()
            .filter(|element| !self.matcher.is_match(element))
            .cloned()
            .collect::<Vec<_>>();
        self.inner.append_comment(filtered_path.as_slice(), text);
    }

    fn reset(&mut self) -> Self::Output {
        self.inner.reset()
    }
}

impl<Handle: Copy + Eq, A: HtmlSink<Handle>, B: HtmlSink<Handle>> HtmlSink<Handle> for (A, B) {
    type Output = (A::Output, B::Output);

    fn append_doctype_to_document(
        &mut self,
        name: &html5ever::tendril::StrTendril,
        public_id: &html5ever::tendril::StrTendril,
        system_id: &html5ever::tendril::StrTendril,
    ) {
        self.0
            .append_doctype_to_document(name, public_id, system_id);
        self.1
            .append_doctype_to_document(name, public_id, system_id);
    }

    fn append_element(
        &mut self,
        context: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    ) {
        self.0.append_element(context, element);
        self.1.append_element(context, element);
    }

    fn append_text(&mut self, context: HtmlContext<Handle>, text: &str) {
        self.0.append_text(context, text);
        self.1.append_text(context, text);
    }

    fn append_comment(&mut self, context: HtmlContext<Handle>, text: &str) {
        self.0.append_comment(context, text);
        self.1.append_comment(context, text);
    }

    fn reset(&mut self) -> Self::Output {
        (self.0.reset(), self.1.reset())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use html5ever::{
        serialize::{SerializeOpts, TraversalScope},
        tendril::TendrilSink,
        ParseOpts,
    };

    fn stream_doc(test: &str, sink: impl HtmlSink<u32>) {
        let mut opts = ParseOpts::default();
        opts.tree_builder.exact_errors = true;
        let parser = parse_document(sink, opts);
        parser.one(test).unwrap();
    }

    fn serialiser(buf: &mut Vec<u8>) -> HtmlSerializer<&mut Vec<u8>, u32> {
        let opts = SerializeOpts::default();
        HtmlSerializer::new(buf, opts)
    }

    #[test]
    fn doc_identity() {
        let mut buf = Vec::new();
        let mut sink = serialiser(&mut buf);
        let test = "<!DOCTYPE html><html><head></head><body><!-- comment --><p><b>hello</b></p><p>world!</p></body></html>";
        stream_doc(test, &mut sink);
        assert_eq!(String::from_utf8(buf).unwrap(), test);
    }

    #[test]
    // #[ignore = "html5ever mysteriously adds a <html> root"]
    fn fragment_identity() {
        let mut buf = Vec::new();
        let mut opts = SerializeOpts::default();
        opts.traversal_scope = TraversalScope::ChildrenOnly(None);
        let mut sink = HtmlSerializer::new(&mut buf, opts);
        let mut opts = ParseOpts::default();
        opts.tree_builder.exact_errors = true;
        let parser = parse_fragment(&mut sink, opts);
        let test = "<p><b>hello</b></p><p>world!</p>";
        parser.one(test).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), test);
    }

    #[test]
    fn remove_elements() {
        let mut buf = Vec::new();
        let mut serializer = serialiser(&mut buf);
        let test = r#"<!DOCTYPE html><html><head></head><body><p class="hello"><!-- comment --><b>hello</b></p><p>world!</p></body></html>"#;
        stream_doc(
            test,
            ElementRemover::wrap(&mut serializer, css_select!(."hello")),
        );
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            r#"<!DOCTYPE html><html><head></head><body><p>world!</p></body></html>"#
        );
    }

    #[test] // for selection, a selected node needs to be appended to the document, if it is not already part of a selected tree. i think for this all to work, either each processor needs to have it's own traversal tree, or maybe, the traversal tree builder from a Sink is only the first step and the processing actually happens using a different interface, probably entirely triggered by appends, but also having a (filtered) access to the tracversal scope
    fn select_element() {
        let mut buf = Vec::new();
        let mut serializer = serialiser(&mut buf);
        let sink = RootFilter::<_, _, _>::wrap(&mut serializer, css_select!("p"));
        let test = "<!DOCTYPE html><html><head></head><body><p><!-- comment --><b>hello</b></p><p>world!</p></body></html>";
        stream_doc(test, sink);
        assert_eq!(buf, b"<p><!-- comment --><b>hello</b></p><p>world!</p>");
    }

    #[test]
    fn extract_data() {}
}
