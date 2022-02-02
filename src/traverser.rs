#![allow(unused)]
use core::fmt;
use std::borrow::Cow;

use html5ever::{
    tree_builder::{NodeOrText, TreeSink},
    *,
};

use crate::{
    css_select,
    selector::{ContextualSelector, ElementSelector, Selector},
    ElementSkipper, HtmlPathElement, HtmlSink,
};

pub fn parse_document<Sink>(
    sink: Sink,
    opts: ParseOpts,
) -> Parser<impl TreeSink<Output = Sink::Output>>
where
    Sink: HtmlSink<u32>,
{
    let sink = ParseTraverser::new_document(sink);
    html5ever::parse_document(sink, opts)
}

pub fn parse_fragment<Sink>(
    sink: Sink,
    opts: ParseOpts,
) -> Parser<impl TreeSink<Output = Sink::Output>>
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

struct ParseTraverser<I> {
    inner: I,
    handle: u32,
    traversal: Vec<TraversalNode>,
    free_nodes: Vec<TraversalNode>,
}

#[derive(Debug)]
struct TraversalNode {
    handle: u32,
    name: html5ever::QualName,
    attrs: Vec<Attribute>,
}
impl TraversalNode {
    pub(crate) fn as_html_path_element(&self) -> HtmlPathElement<u32> {
        HtmlPathElement {
            handle: self.handle,
            name: self.name.clone(),
            attrs: Cow::Borrowed(&self.attrs),
        }
    }
}

impl fmt::Display for TraversalNode {
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
            handle: 0,
            traversal: vec![],
            free_nodes: vec![],
        }
    }
    pub(crate) fn new_fragment(serializer: I) -> Self {
        Self {
            inner: serializer,
            handle: 1,
            traversal: vec![TraversalNode {
                handle: 1,
                name: QualName {
                    prefix: None,
                    ns: ns!(),
                    local: local_name!("body"),
                },
                attrs: vec![],
            }],
            free_nodes: vec![],
        }
    }

    fn node(&self, target: &u32) -> &TraversalNode {
        for node in self.traversal.iter().rev() {
            if &node.handle == target {
                return node;
            }
        }
        for node in self.free_nodes.iter().rev() {
            if &node.handle == target {
                return node;
            }
        }
        panic!("Couldn't find elem with handle {}", target);
    }
}

impl<I: HtmlSink<u32>> TreeSink for ParseTraverser<I> {
    type Handle = u32;

    type Output = I::Output;

    fn finish(self) -> Self::Output {
        self.inner.finish()
    }

    fn parse_error(&mut self, msg: std::borrow::Cow<'static, str>) {
        println!("Parse error : {}", msg);
    }

    fn get_document(&mut self) -> Self::Handle {
        0
    }

    fn elem_name<'a>(&'a self, target: &'a Self::Handle) -> html5ever::ExpandedName<'a> {
        self.node(target).name.expanded()
    }

    fn create_element(
        &mut self,
        name: html5ever::QualName,
        attrs: Vec<html5ever::Attribute>,
        _flags: html5ever::tree_builder::ElementFlags,
    ) -> Self::Handle {
        self.handle += 1;
        self.free_nodes.push(TraversalNode {
            handle: self.handle,
            name,
            attrs,
        });
        self.handle
    }

    fn create_comment(&mut self, text: html5ever::tendril::StrTendril) -> Self::Handle {
        println!("Ignoring html comment, inserting span instead : {}", text);
        self.handle += 1;
        self.free_nodes.push(TraversalNode {
            handle: self.handle,
            name: QualName {
                prefix: None,
                ns: ns!(html),
                local: local_name!("span"),
            },
            attrs: vec![],
        });
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
            match child {
                NodeOrText::AppendNode(handle) => {
                    let child_index = self
                        .free_nodes
                        .iter()
                        .enumerate()
                        .rev()
                        .find_map(|(index, node)| (handle == node.handle).then(|| index))
                        .unwrap();
                    let element = self.free_nodes.remove(child_index);
                    assert_eq!(element.handle, handle);
                    // println!(
                    //     "appending child {} to {}",
                    //     &element,
                    //     parent.map_or(String::new(), TraversalNode::to_string)
                    // );
                    self.inner.append_element(
                        &self
                            .traversal
                            .iter()
                            .map(TraversalNode::as_html_path_element)
                            .collect::<Vec<_>>(), // TODO these should be reused
                        &element.as_html_path_element(),
                    );
                    self.traversal.push(element);
                }
                NodeOrText::AppendText(text) => {
                    // println!(
                    //     "appending child \"{}\" to {}",
                    //     text.to_string(),
                    //     parent.map_or(String::new(), TraversalNode::to_string)
                    // );
                    self.inner.append_text(
                        &self
                            .traversal
                            .iter()
                            .map(TraversalNode::as_html_path_element)
                            .collect::<Vec<_>>(),
                        &text,
                    );
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
        println!("Quirks mode : {:?}", mode);
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
