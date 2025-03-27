use html5ever::{tendril::StrTendril, *};

use crate::{HtmlContext, HtmlPathElement};

/// Selects elements using a syntax similar to css 1 selectors, supporting css 1 selectors except pseudo-elements and pseudo classes
///
/// ```
/// use html5streams::css_select;
///
/// css_select!("p");
/// css_select!("p"."quote");
/// css_select!(."quote");
/// css_select!(#"main");
/// css_select!((#"main") ("p"."quote"));
/// ```
#[macro_export]
macro_rules! css_select {
    (@inner [($($head:tt)+)] -> [$selector:expr]) => {
        ($crate::selector::ContextualSelectCons($selector , css_select!($($head)+)))
    };
    (@inner [($($head:tt)+) $($tail:tt)*] -> [$selector:expr]) => {
        css_select!(@inner [$($tail)*] -> [$crate::selector::ContextSelectCons($selector , css_select!($($head)+))])
    };
    ($(($($selectors:tt)+))+) => {
        css_select!(@inner [$(($($selectors)+))+] -> [$crate::selector::MatchAll])
    };
    ($name:literal.$class:literal) => {
        $crate::selector::Selector::and(
            $crate::selector::NameSelector($name),
            $crate::selector::ClassSelector($class),
        )
    };
    ($name:literal#$id:literal) => {
        $crate::selector::Selector::and(
            $crate::selector::NameSelector($name),
            $crate::selector::IdSelector($id),
        )
    };
    ($name:literal) => {
        $crate::selector::NameSelector($name)
    };
    (.$class:literal) => {
        $crate::selector::ClassSelector($class)
    };
    (#$id:literal) => {
        $crate::selector::IdSelector($id)
    };
}

pub trait Selector {
    fn is_match<Handle>(&self, element: &HtmlPathElement<'_, Handle>) -> bool;

    fn and<O: Selector>(self, other: O) -> AndSelector<Self, O>
    where
        Self: Sized,
    {
        AndSelector(self, other)
    }
}

pub trait ContextualSelector {
    fn context_match<Handle>(
        &self,
        context: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    ) -> bool;

    fn or<O: ContextualSelector>(self, other: O) -> GroupSelector<Self, O>
    where
        Self: Sized,
    {
        GroupSelector(self, other)
    }
}

pub trait OnlyContextualSelector {
    fn context_match<Handle>(&self, context: HtmlContext<'_, Handle>) -> bool;
}

impl<S> ContextualSelector for S
where
    S: Selector,
{
    fn context_match<Handle>(
        &self,
        _context: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    ) -> bool {
        self.is_match(element)
    }
}

pub struct NameSelector(pub &'static str);

impl Selector for NameSelector {
    fn is_match<Handle>(&self, element: &HtmlPathElement<'_, Handle>) -> bool {
        *self.0 == *element.name.local
    }
}

pub struct ClassSelector(pub &'static str);

impl Selector for ClassSelector {
    fn is_match<Handle>(&self, element: &HtmlPathElement<'_, Handle>) -> bool {
        element.classes().any(|class| class == self.0)
    }
}

pub struct IdSelector(pub &'static str);

impl Selector for IdSelector {
    fn is_match<Handle>(&self, element: &HtmlPathElement<'_, Handle>) -> bool {
        const ID: QualName = QualName {
            prefix: None,
            ns: ns!(),
            local: local_name!("id"),
        };
        if let Some(id) = element.attr(ID) {
            let var_name: &str = id;
            self.0 == var_name
        } else {
            false
        }
    }
}

#[derive(Debug, Default)]
pub struct ElementSelector {
    name: Option<QualName>,
    id: Option<StrTendril>,
    classes: Vec<StrTendril>,
}

impl ElementSelector {
    pub fn element_match<Handle>(&self, element: &HtmlPathElement<'_, Handle>) -> bool {
        const ID: QualName = QualName {
            prefix: None,
            ns: ns!(),
            local: local_name!("id"),
        };
        self.name
            .as_ref()
            .is_none_or(|match_name| *match_name == element.name)
            && self
                .id
                .as_ref()
                .is_none_or(|match_id| element.attr(ID) == Some(match_id))
            && self
                .classes
                .iter()
                .all(|match_class| element.classes().any(|class| **match_class == *class))
    }

    pub fn class(self, class: StrTendril) -> Self {
        let mut classes = self.classes;
        classes.push(class);
        Self {
            name: self.name,
            id: self.id,
            classes,
        }
    }

    pub fn name(self, local_name: LocalName) -> Self {
        Self {
            name: Some(QualName {
                prefix: None,
                ns: ns!(html),
                local: local_name,
            }),
            id: self.id,
            classes: self.classes,
        }
    }
}

impl Selector for ElementSelector {
    fn is_match<Handle>(&self, element: &HtmlPathElement<'_, Handle>) -> bool {
        self.element_match(element)
    }
}

/// A contextual selector, the last selector must match the element exactly and the preceding must match elements in the context in that order
impl<S: Selector> ContextualSelector for [S] {
    fn context_match<Handle>(
        &self,
        path: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    ) -> bool {
        let mut to_match = self.iter().rev();
        if let Some(end_matcher) = to_match.next() {
            if !end_matcher.is_match(element) {
                return false;
            }
        } else {
            return true;
        }
        let mut path = path.iter().rev();
        'outer: for matcher in to_match {
            for element in &mut path {
                if matcher.is_match(element) {
                    continue 'outer;
                }
            }
            return false;
        }
        true
    }
}

/// Always matches
pub struct MatchAll;

impl Selector for MatchAll {
    fn is_match<Handle>(&self, _element: &HtmlPathElement<'_, Handle>) -> bool {
        true
    }
}

impl OnlyContextualSelector for MatchAll {
    fn context_match<Handle>(&self, _context: HtmlContext<'_, Handle>) -> bool {
        true
    }
}

/// Matches something in the context, then continues by using the second matcher for the remaining context
pub struct ContextSelectCons<C, A>(pub C, pub A);

impl<C: OnlyContextualSelector, A: Selector> OnlyContextualSelector for ContextSelectCons<C, A> {
    fn context_match<Handle>(&self, mut context: HtmlContext<'_, Handle>) -> bool {
        while let Some((last, rest)) = context.split_last() {
            let element = last;
            context = rest;
            if self.1.is_match(element) {
                return self.0.context_match(context);
            }
        }
        false
    }
}

/// Matches the element, then continues by using the second matcher for the remaining context
pub struct ContextualSelectCons<C: OnlyContextualSelector, A: Selector>(pub C, pub A);

impl<C: OnlyContextualSelector, A: Selector> ContextualSelector for ContextualSelectCons<C, A> {
    fn context_match<'a, Handle>(
        &self,
        context: HtmlContext<'a, Handle>,
        element: &'a HtmlPathElement<'a, Handle>,
    ) -> bool {
        self.1.is_match(element) && self.0.context_match(context)
    }
}

/// Groups together 2 selectors, selects elements that either would select
pub struct GroupSelector<A: ContextualSelector, B: ContextualSelector>(A, B);

impl<A: ContextualSelector, B: ContextualSelector> ContextualSelector for GroupSelector<A, B> {
    fn context_match<Handle>(
        &self,
        path: HtmlContext<'_, Handle>,
        element: &HtmlPathElement<'_, Handle>,
    ) -> bool {
        self.0.context_match(path, element) || self.1.context_match(path, element)
    }
}

/// COmbines 2 selectors, selecting something taht both would select
pub struct AndSelector<A: Selector, B: Selector>(A, B);

impl<A: Selector, B: Selector> Selector for AndSelector<A, B> {
    fn is_match<Handle>(&self, element: &HtmlPathElement<'_, Handle>) -> bool {
        self.0.is_match(element) && self.1.is_match(element)
    }
}

#[test]
fn test_matchers() {
    let mut handle = 0;
    let mut el = |local, attrs: Vec<Attribute>| {
        handle += 1;
        HtmlPathElement {
            handle,
            name: QualName {
                prefix: None,
                ns: ns!(html),
                local,
            },
            attrs: attrs.into(),
        }
    };
    let attr = |local, value: &str| Attribute {
        name: QualName {
            prefix: None,
            ns: ns!(),
            local,
        },
        value: value.into(),
    };
    let el_main = el(local_name!("div"), vec![attr(local_name!("id"), "main")]);
    let el_p = el(local_name!("p"), vec![]);
    let el_quote = el(
        local_name!("p"),
        vec![Attribute {
            name: QualName {
                prefix: None,
                ns: ns!(),
                local: local_name!("class"),
            },
            value: "fixed quote".into(),
        }],
    );
    let path_body = [
        el(local_name!("html"), vec![]),
        el(local_name!("html"), vec![]),
    ];
    let path_main = [
        el(local_name!("html"), vec![]),
        el(local_name!("html"), vec![]),
        el_main.clone(),
    ];

    assert!(css_select!("p").context_match(&path_main, &el_p));
    assert!(css_select!("p").context_match(&path_main, &el_quote));
    assert!(!css_select!("p").context_match(&path_body, &el_main));

    assert!(!css_select!("p"."quote").context_match(&path_main, &el_p));
    assert!(css_select!("p"."quote").context_match(&path_main, &el_quote));
    assert!(!css_select!("p"."quote").context_match(&path_body, &el_main));

    assert!(!css_select!(."quote").context_match(&path_main, &el_p));
    assert!(css_select!(."quote").context_match(&path_main, &el_quote));
    assert!(!css_select!(."quote").context_match(&path_body, &el_main));

    assert!(!css_select!(#"main").context_match(&path_main, &el_p));
    assert!(!css_select!(#"main").context_match(&path_main, &el_quote));
    assert!(css_select!(#"main").context_match(&path_body, &el_main));

    assert!(!css_select!((#"main") ("p"."quote")).context_match(&path_main, &el_p));
    assert!(css_select!((#"main") ("p"."quote")).context_match(&path_main, &el_quote));
    assert!(!css_select!((#"main") ("p"."quote")).context_match(&path_body, &el_main));
    assert!(!css_select!((#"main") ("p"."quote")).context_match(&path_body, &el_quote));
}
