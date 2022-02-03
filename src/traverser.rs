#![allow(unused)]
use core::fmt;
use std::{borrow::Cow, collections::HashMap};

use html5ever::{
    tree_builder::{NodeOrText, TreeSink},
    *,
};

use crate::{
    css_select,
    selector::{ContextualSelector, ElementSelector, NameSelector, Selector},
    ElementSkipper, HtmlPathElement, HtmlSink,
};

pub fn parse_document<Sink>(sink: Sink, opts: ParseOpts) -> Parser<ParseTraverser<Sink>>
where
    Sink: HtmlSink<u32>,
{
    let sink = ParseTraverser::new_document(sink);
    html5ever::parse_document(sink, opts)
}

pub fn parse_fragment<Sink>(
    sink: Sink,
    opts: ParseOpts,
) -> Parser<ParseTraverser<ElementSkipper<Sink, NameSelector>>>
where
    Sink: HtmlSink<u32>,
{
    let context_name = QualName {
        prefix: None,
        ns: ns!(html),
        local: local_name!("body"),
    };
    let context_attrs = vec![];
    let sink = ParseTraverser::new_fragment(ElementSkipper::wrap(sink, css_select!("html"))); // TODO find a way to do this without skipping filter
    html5ever::parse_fragment(sink, opts, context_name, context_attrs)
}

pub struct ParseTraverser<I> {
    inner: I,
    parse_error: Option<Cow<'static, str>>,
    handle: u32,
    traversal: Vec<TraversalElement>,
    free_nodes: HashMap<u32, Node>,
}

#[derive(Debug)]
enum Node {
    Element(TraversalElement),
    Comment(html5ever::tendril::StrTendril),
}

#[derive(Debug)]
struct TraversalElement {
    handle: u32,
    name: html5ever::QualName,
    attrs: Vec<Attribute>,
}
impl TraversalElement {
    pub(crate) fn as_html_path_element(&self) -> HtmlPathElement<u32> {
        HtmlPathElement {
            handle: self.handle,
            name: self.name.clone(),
            attrs: Cow::Borrowed(&self.attrs),
        }
    }
}

impl fmt::Display for TraversalElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{}: <{}{}>",
            self.handle,
            self.name.local,
            self.attrs
                .iter()
                .map(|att| format!(" {}=\"{}\"", att.name.local, &*att.value))
                .collect::<String>()
        )
    }
}

impl<I> ParseTraverser<I> {
    pub(crate) fn new_document(serializer: I) -> Self {
        Self {
            inner: serializer,
            parse_error: None,
            handle: 0,
            traversal: vec![],
            free_nodes: HashMap::new(),
        }
    }
    pub(crate) fn new_fragment(serializer: I) -> Self {
        Self {
            inner: serializer,
            parse_error: None,
            handle: 1,
            traversal: vec![TraversalElement {
                handle: 1,
                name: QualName {
                    prefix: None,
                    ns: ns!(),
                    local: local_name!("body"),
                },
                attrs: vec![],
            }],
            free_nodes: HashMap::new(),
        }
    }

    fn element(&self, target: &u32) -> &TraversalElement {
        for element in self.traversal.iter().rev() {
            if &element.handle == target {
                return element;
            }
        }
        if let Some(Node::Element(element)) = self.free_nodes.get(target) {
            return element;
        }
        panic!("Couldn't find elem with handle {}", target);
    }
}

impl<I: HtmlSink<u32>> TreeSink for ParseTraverser<I> {
    type Handle = u32;

    type Output = Result<I::Output, Cow<'static, str>>;

    fn finish(self) -> Self::Output {
        if let Some(err) = self.parse_error {
            Err(err)
        } else {
            Ok(self.inner.finish())
        }
    }

    fn parse_error(&mut self, msg: Cow<'static, str>) {
        // currently using a fast fail mode, ideally we'd tell html5ever to abort the parse
        self.parse_error = Some(msg);
    }

    fn get_document(&mut self) -> Self::Handle {
        0
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> html5ever::ExpandedName<'a> {
        self.element(target).name.expanded()
    }

    fn create_element(
        &mut self,
        name: html5ever::QualName,
        attrs: Vec<html5ever::Attribute>,
        _flags: html5ever::tree_builder::ElementFlags,
    ) -> Self::Handle {
        self.handle += 1;
        self.free_nodes.insert(
            self.handle,
            Node::Element(TraversalElement {
                handle: self.handle,
                name,
                attrs,
            }),
        );
        self.handle
    }

    fn create_comment(&mut self, text: html5ever::tendril::StrTendril) -> Self::Handle {
        self.handle += 1;
        self.free_nodes.insert(self.handle, Node::Comment(text));
        self.handle
    }

    fn create_pi(
        &mut self,
        target: html5ever::tendril::StrTendril,
        data: html5ever::tendril::StrTendril,
    ) -> Self::Handle {
        todo!()
    }

    fn append(&mut self, parent: &Self::Handle, child: NodeOrText<Self::Handle>) {
        if *parent == self.get_document()
            || self
                .traversal
                .iter()
                .rev()
                .any(|node| parent == &node.handle)
        {
            // pop traversal back to parent
            let parent = loop {
                if self.traversal.last().map_or(0, |t| t.handle) == *parent {
                    break self.traversal.last();
                } else {
                    self.traversal.pop();
                }
            };
            if self.parse_error.is_none() {
                match child {
                    NodeOrText::AppendNode(handle) => {
                        let node = self.free_nodes.remove(&handle).unwrap();
                        let context = self
                            .traversal
                            .iter()
                            .map(TraversalElement::as_html_path_element)
                            .collect::<Vec<_>>(); // TODO these should be reused;
                        match node {
                            Node::Element(element) => {
                                assert_eq!(element.handle, handle);
                                self.inner
                                    .append_element(&context, &element.as_html_path_element());
                                self.traversal.push(element);
                            }
                            Node::Comment(text) => {
                                self.inner.append_comment(&context, &text);
                            }
                        }
                    }
                    NodeOrText::AppendText(text) => {
                        self.inner.append_text(
                            &self
                                .traversal
                                .iter()
                                .map(TraversalElement::as_html_path_element)
                                .collect::<Vec<_>>(),
                            &text,
                        );
                    }
                }
            }
        }
    }

    fn append_based_on_parent_node(
        &mut self,
        element: &Self::Handle,
        prev_element: &Self::Handle,
        child: html5ever::tree_builder::NodeOrText<Self::Handle>,
    ) {
        todo!()
    }

    fn append_doctype_to_document(
        &mut self,
        name: html5ever::tendril::StrTendril,
        public_id: html5ever::tendril::StrTendril,
        system_id: html5ever::tendril::StrTendril,
    ) {
        self.inner
            .append_doctype_to_document(&name, &public_id, &system_id)
    }

    fn get_template_contents(&mut self, target: &Self::Handle) -> Self::Handle {
        todo!()
    }

    fn same_node(&self, x: &Self::Handle, y: &Self::Handle) -> bool {
        // not sure what to do here
        x == y
    }

    fn set_quirks_mode(&mut self, mode: html5ever::tree_builder::QuirksMode) {
        // println!("Quirks mode : {:?}", mode);
    }

    fn append_before_sibling(
        &mut self,
        sibling: &Self::Handle,
        new_node: html5ever::tree_builder::NodeOrText<Self::Handle>,
    ) {
        todo!()
    }

    fn add_attrs_if_missing(&mut self, target: &Self::Handle, attrs: Vec<html5ever::Attribute>) {
        todo!()
    }

    fn remove_from_parent(&mut self, target: &Self::Handle) {
        todo!()
    }

    fn reparent_children(&mut self, node: &Self::Handle, new_parent: &Self::Handle) {
        todo!()
    }
}
