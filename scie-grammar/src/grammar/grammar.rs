use core::cmp;
use scie_scanner::scanner::scie_scanner::IOnigCaptureIndex;
use std::collections::HashMap as Map;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use crate::grammar::line_tokens::{IToken, LineTokens, TokenTypeMatcher};
use crate::grammar::local_stack_element::LocalStackElement;
use crate::grammar::rule_container::RuleContainer;
use crate::grammar::{MatchRuleResult, ScopeListElement, StackElement};
use crate::inter::{IRawGrammar, IRawRepository, IRawRepositoryMap, IRawRule};
use crate::rule::abstract_rule::RuleEnum;
use crate::rule::rule_factory::RuleFactory;
use crate::rule::{
    AbstractRule, BeginEndRule, BeginWhileRule, EmptyRule, IGrammarRegistry, IRuleRegistry,
};

pub trait Matcher {}

#[derive(Debug, Clone)]
pub struct CheckWhileRuleResult {
    pub rule: BeginWhileRule,
    pub stack: StackElement,
}

#[derive(Debug, Clone)]
pub struct CheckWhileConditionResult {
    pub stack: StackElement,
    pub line_pos: i32,
    pub anchor_position: i32,
    pub is_first_line: bool,
}

#[derive(Debug, Clone)]
pub struct TokenizeResult {
    pub tokens: Vec<IToken>,
    pub rule_stack: Option<StackElement>,
}

#[derive(Debug, Clone)]
pub struct Grammar {
    root_id: i32,
    pub grammar: IRawGrammar,
    pub last_rule_id: i32,
    rules: Vec<Box<dyn AbstractRule>>,
    pub _empty_rule: Map<i32, Box<dyn AbstractRule>>,
    pub rule_container: Box<RuleContainer>,
    pub scope_name_map: Map<String, i32>,
    pub _token_type_matchers: Vec<TokenTypeMatcher>,
}

pub fn init_grammar(raw_grammar: IRawGrammar, _base: Option<IRawRule>) -> IRawGrammar {
    let mut grammar = raw_grammar.to_owned();

    let mut new_based: IRawRule = IRawRule::new();
    if raw_grammar.repository.is_some() {
        new_based.location = raw_grammar.repository.clone().unwrap().location;
    }
    new_based.patterns = Some(raw_grammar.patterns.clone());
    new_based.name = raw_grammar.scope_name.clone();

    let mut repository_map = IRawRepositoryMap::new();
    repository_map.base_s = Some(Box::from(new_based.clone()));
    repository_map.self_s = Some(Box::from(new_based.clone()));
    if raw_grammar.repository.is_some() {
        repository_map.name_map = raw_grammar.repository.unwrap().clone().map.name_map;
    }

    grammar.repository = Some(IRawRepository {
        map: Box::new(repository_map.clone()),
        location: None,
    });

    grammar
}

impl Grammar {
    pub fn new(raw_grammar: IRawGrammar) -> Self {
        let inited_grammar = init_grammar(raw_grammar, None);

        let mut _empty_rule = Map::new();

        let mut grammar = Grammar {
            last_rule_id: 0,
            grammar: inited_grammar,
            root_id: -1,
            rule_container: Box::new(Default::default()),
            scope_name_map: Map::new(),
            _token_type_matchers: vec![],
            _empty_rule,
            rules: vec![],
        };

        grammar._empty_rule.insert(-2, Box::new(EmptyRule {}));
        grammar
    }

    fn tokenize(
        &mut self,
        line_text: &str,
        prev_state: &mut Option<StackElement>,
        emit_binary_tokens: bool,
    ) -> TokenizeResult {
        if self.root_id == -1 {
            let repository = self.grammar.repository.clone().unwrap();
            let based = repository.clone().map.self_s.unwrap();
            self.root_id = RuleFactory::get_compiled_rule_id(
                *based.clone(),
                self,
                &mut repository.clone(),
                "",
            );

            for (id, rule) in self.rule_container.rule_id2desc.iter() {
                if rule.get_rule()._name.is_some() {
                    self.scope_name_map
                        .insert(rule.get_rule()._name.as_ref().unwrap().clone(), *id);
                }
            }
        }

        let mut is_first_line: bool = false;

        let mut current_state = StackElement::null();
        match prev_state.clone() {
            None => is_first_line = true,
            Some(state) => {
                if state == StackElement::null() {
                    is_first_line = true
                }

                current_state = state;
            }
        }

        if is_first_line {
            let _root_scope_name = self.get_rule(self.root_id).get_name(None, None);
            let mut root_scope_name = String::from("unknown");
            if let Some(name) = _root_scope_name {
                root_scope_name = name
            }

            let scope_list = ScopeListElement::new(None, root_scope_name);
            let state = StackElement::new(
                None,
                self.root_id,
                -1,
                -1,
                false,
                None,
                scope_list.clone(),
                scope_list,
            );

            current_state = state;
        } else {
            is_first_line = false;
            current_state.reset();
        }

        let format_line_text: String = String::from(line_text) + "\n";
        let mut line_tokens = LineTokens::new(
            emit_binary_tokens,
            line_text,
            self._token_type_matchers.clone(),
        );

        let line_length = format_line_text.len();
        let next_state = self.tokenize_string(
            &*format_line_text,
            is_first_line,
            0,
            current_state,
            &mut line_tokens,
            true,
        );

        let stack = &mut next_state.clone().unwrap();
        let vec = line_tokens.get_result(stack, line_length as i32);
        TokenizeResult {
            tokens: vec.clone(),
            rule_stack: next_state,
        }
    }

    pub fn tokenize_string<'a>(
        &mut self,
        line_text: &'a str,
        mut is_first_line: bool,
        mut line_pos: i32,
        mut stack: StackElement,
        line_tokens: &mut LineTokens,
        check_while_conditions: bool,
    ) -> Option<StackElement> {
        let line_length = line_text.len().clone();
        let mut _stop = false;
        let mut anchor_position = -1;

        if check_while_conditions {
            let while_check_result =
                self.check_while_conditions(line_text, is_first_line, line_pos, stack, line_tokens);
            stack = while_check_result.stack;
            line_pos = while_check_result.line_pos;
            is_first_line = while_check_result.is_first_line;
            anchor_position = while_check_result.anchor_position;
        }

        while !_stop {
            let r = self.match_rule(
                line_text,
                is_first_line,
                line_pos,
                &mut stack,
                anchor_position,
            );
            if let None = r {
                line_tokens.produce(&mut stack, line_length as i32);
                _stop = true;
                return Some(stack);
            }

            let capture_result = r.unwrap();
            let capture_indices = capture_result.capture_indices;
            let matched_rule_id = capture_result.matched_rule_id;
            if matched_rule_id == -1 {
                let _popped_rule = self.get_rule(stack.rule_id);
                if _popped_rule.get_rule()._type == "BeginEndRule" {
                    let popped_rule = _popped_rule
                        .get_instance()
                        .downcast_ref::<BeginEndRule>()
                        .unwrap();
                    let name_scopes_list = stack.name_scopes_list.clone();
                    line_tokens.produce(&mut stack, capture_indices[0].start as i32);

                    stack = stack.set_content_name_scopes_list(name_scopes_list);
                    let end_captures = &popped_rule.end_captures.clone();
                    Grammar::handle_captures(
                        self,
                        line_text,
                        is_first_line,
                        &mut stack,
                        line_tokens,
                        end_captures,
                        &capture_indices,
                    );

                    line_tokens.produce(&mut stack, capture_indices[0].end as i32);
                    let popped_anchor_pos = stack.anchor_pos.clone();
                    if let Some(_stack) = stack.pop() {
                        stack = _stack;
                    }
                    anchor_position = popped_anchor_pos;
                } else {
                    println!("_popped_rule {:?}", _popped_rule.clone());
                    _stop = true;
                    return Some(stack);
                }
            } else {
                let rule = self.get_rule(matched_rule_id);
                line_tokens.produce(&mut stack, capture_indices[0].start as i32);
                let scope_name =
                    rule.get_name(Some(String::from(line_text)), Some(&capture_indices));
                let name_scopes_list = stack.content_name_scopes_list.push(scope_name);
                let mut begin_rule_capture_eol = false;
                if capture_indices[0].end == line_length {
                    begin_rule_capture_eol = true;
                }
                stack = stack.push(
                    matched_rule_id,
                    line_pos,
                    anchor_position,
                    begin_rule_capture_eol,
                    None,
                    name_scopes_list.clone(),
                    name_scopes_list.clone(),
                );

                match rule.get_rule_instance() {
                    RuleEnum::BeginEndRule(rule) => {
                        let begin_rule = rule.clone();
                        Grammar::handle_captures(
                            self,
                            line_text,
                            is_first_line,
                            &mut stack,
                            line_tokens,
                            &begin_rule.begin_captures,
                            &capture_indices,
                        );

                        line_tokens.produce(&mut stack, capture_indices[0].end as i32);
                        anchor_position = capture_indices[0].end as i32;
                        let content_name = begin_rule.get_content_name(
                            Some(String::from(line_text)),
                            Some(&capture_indices),
                        );
                        let _content_name_scopes_list = name_scopes_list.push(content_name);
                        stack = stack.set_content_name_scopes_list(_content_name_scopes_list);

                        if begin_rule.end_has_back_references {
                            stack = stack.set_end_rule(
                                begin_rule.get_end_with_resolved_back_references(
                                    line_text,
                                    capture_indices.clone(),
                                ),
                            );
                        }
                    }
                    RuleEnum::BeginWhileRule(rule) => {
                        let push_rule = rule.clone();
                        Grammar::handle_captures(
                            self,
                            line_text,
                            is_first_line,
                            &mut stack,
                            line_tokens,
                            &push_rule.begin_captures,
                            &capture_indices,
                        );

                        line_tokens.produce(&mut stack, capture_indices[0].end as i32);
                        anchor_position = capture_indices[0].end as i32;
                        let content_name = push_rule.get_content_name(
                            Some(String::from(line_text)),
                            Some(&capture_indices),
                        );

                        let content_name_scopes_list = name_scopes_list.push(content_name);
                        stack = stack.set_content_name_scopes_list(content_name_scopes_list);
                    }
                    RuleEnum::MatchRule(match_rule) => {
                        let captures = &match_rule.captures.clone();
                        Grammar::handle_captures(
                            self,
                            line_text,
                            is_first_line,
                            &mut stack,
                            line_tokens,
                            captures,
                            &capture_indices,
                        );
                        line_tokens.produce(&mut stack, capture_indices[0].end as i32);
                        if let Some(_stack) = stack.pop() {
                            stack = _stack;
                        }
                    }
                    _ => {
                        panic!("todo: RuleEnum - Others");
                    }
                }
            }

            if capture_indices[0].end > line_pos as usize {
                line_pos = capture_indices[0].end as i32;
                is_first_line = false;
            }
        }
        Some(stack)
    }

    pub fn handle_captures<'a>(
        grammar: &mut Grammar,
        line_text: &'a str,
        is_first_line: bool,
        stack: &mut StackElement,
        line_tokens: &'a mut LineTokens,
        captures: &Vec<Box<dyn AbstractRule>>,
        capture_indices: &Vec<IOnigCaptureIndex>,
    ) {
        if captures.len() == 0 {
            return;
        }

        let len = cmp::min(captures.len(), capture_indices.len());
        let mut local_stack: Vec<LocalStackElement> = vec![];
        let max_end = capture_indices[0].end;
        for i in 0..len {
            if let RuleEnum::CaptureRule(capture) = captures[i].get_rule_instance() {
                let capture_index = &capture_indices[i];
                if capture_index.length == 0 {
                    continue;
                }

                if capture_index.start > max_end {
                    continue;
                }

                while local_stack.len() > 0
                    && local_stack[local_stack.len() - 1].end_pos <= capture_index.start as i32
                {
                    let local_stack_element = &local_stack[local_stack.len() - 1];
                    line_tokens.produce_from_scopes(
                        &local_stack_element.scopes,
                        local_stack_element.end_pos,
                    );
                    local_stack.pop();
                }

                if local_stack.len() > 0 {
                    let local_stack_element = &local_stack[local_stack.len() - 1];
                    line_tokens.produce_from_scopes(
                        &local_stack_element.scopes,
                        capture_index.start as i32,
                    );
                } else {
                    line_tokens.produce(stack, capture_index.start as i32);
                }

                if capture.retokenize_captured_with_rule_id != 0 {
                    let scope_name =
                        capture.get_name(Some(String::from(line_text)), Some(&capture_indices));
                    let name_scopes_list = stack.content_name_scopes_list.push(scope_name);
                    let content_name = capture
                        .get_content_name(Some(String::from(line_text)), Some(&capture_indices));
                    let content_name_scopes_list = name_scopes_list.push(content_name);

                    let stack_clone = stack.clone().push(
                        capture.retokenize_captured_with_rule_id,
                        capture_index.start as i32,
                        -1,
                        false,
                        None,
                        name_scopes_list,
                        content_name_scopes_list,
                    );

                    let sub_text = line_text.split_at(capture_index.end).0;
                    let mut sub_is_first_line = false;
                    if is_first_line && capture_index.start == 0 {
                        sub_is_first_line = true;
                    }
                    Grammar::tokenize_string(
                        grammar,
                        sub_text,
                        sub_is_first_line,
                        capture_index.start as i32,
                        stack_clone,
                        line_tokens,
                        false,
                    );
                    continue;
                }

                let capture_scope_name =
                    captures[i].get_name(Some(String::from(line_text)), Some(&capture_indices));
                if capture_scope_name.is_some() {
                    let mut base = &stack.content_name_scopes_list;
                    if local_stack.len() > 0 {
                        base = &local_stack[local_stack.len() - 1].scopes;
                    }
                    let capture_rule_scopes_list = base.push(capture_scope_name);
                    local_stack.push(LocalStackElement::new(
                        capture_rule_scopes_list,
                        capture_index.end as i32,
                    ));
                }
            } else {
                println!("lose rule: {:?}", captures[i].clone());
            }
        }

        while local_stack.len() > 0 {
            let last_stack = &local_stack[local_stack.len() - 1];
            line_tokens.produce_from_scopes(&last_stack.scopes, last_stack.end_pos);
            local_stack.pop();
        }
    }
    /**
     * Walk the stack from bottom to top, and check each while condition in this order.
     * If any fails, cut off the entire stack above the failed while condition. While conditions
     * may also advance the linePosition.
     */
    pub fn check_while_conditions(
        &mut self,
        line_text: &str,
        mut is_first_line: bool,
        mut line_pos: i32,
        mut stack: StackElement,
        line_tokens: &mut LineTokens,
    ) -> CheckWhileConditionResult {
        let mut anchor_position = -1;
        if stack.begin_rule_captured_eol {
            anchor_position = 0
        }
        let mut while_rules = vec![];
        let mut has_node = true;
        let mut node = stack.clone();
        while has_node {
            let rule = self.get_rule(node.rule_id);
            if rule.get_rule()._type == "BeginWhileRule" {
                if let RuleEnum::BeginWhileRule(begin_while_rule) = rule.get_rule_instance() {
                    while_rules.push(CheckWhileRuleResult {
                        rule: begin_while_rule.clone(),
                        stack: node.clone(),
                    })
                }
            }

            match node.pop() {
                None => has_node = false,
                Some(n) => {
                    node = n;
                }
            }
        }

        for mut while_rule in while_rules {
            let mut rule_scanner = while_rule.rule.compile_while(
                while_rule.stack.end_rule.clone(),
                is_first_line,
                anchor_position == line_pos,
            );
            let match_result = rule_scanner
                .scanner
                .find_next_match_sync(line_text, line_pos);

            match match_result {
                None => {
                    stack = while_rule.stack.pop().unwrap();
                    break;
                }
                Some(r) => {
                    let matched_rule_id = rule_scanner.rules[r.index];
                    if matched_rule_id != -2 {
                        // we shouldn't end up here
                        stack = while_rule.stack.pop().unwrap();
                        break;
                    }

                    if r.capture_indices.len() > 0 {
                        line_tokens
                            .produce(&mut while_rule.stack, r.capture_indices[0].start as i32);
                        Grammar::handle_captures(
                            self,
                            line_text,
                            is_first_line,
                            &mut while_rule.stack,
                            line_tokens,
                            &while_rule.rule.while_captures,
                            &r.capture_indices,
                        );
                        let end_index = r.capture_indices[0].end;
                        line_tokens.produce(&mut while_rule.stack, end_index as i32);
                        anchor_position = end_index as i32;
                        if end_index > line_pos as usize {
                            line_pos = end_index as i32;
                            is_first_line = false;
                        }
                    }
                }
            }
        }

        CheckWhileConditionResult {
            stack,
            line_pos,
            anchor_position,
            is_first_line,
        }
    }

    pub fn match_rule<'a>(
        &mut self,
        line_text: &'a str,
        is_first_line: bool,
        line_pos: i32,
        stack: &mut StackElement,
        anchor_position: i32,
    ) -> Option<MatchRuleResult> {
        let mut rule_scanner =
            self.rule_container
                .compile_rule(stack, is_first_line, line_pos == anchor_position);

        let r = rule_scanner
            .scanner
            .find_next_match_sync(line_text, line_pos);

        if let Some(result) = r {
            let match_rule_result = MatchRuleResult {
                capture_indices: result.capture_indices,
                matched_rule_id: rule_scanner.rules[result.index],
            };

            Some(match_rule_result)
        } else {
            None
        }
    }

    pub fn tokenize_line(
        &mut self,
        line_text: &str,
        prev_state: &mut Option<StackElement>,
    ) -> TokenizeResult {
        self.tokenize(line_text, prev_state, false)
    }

    pub fn dispose(&self) {
        for (_key, _rule) in self.rule_container.rule_id2desc.iter() {
            // rule.dispose();
        }
    }

    pub fn from_file(grammar_path: &str) -> Self {
        let path = Path::new(grammar_path);
        let mut file = File::open(path).unwrap();
        let mut data = String::new();
        file.read_to_string(&mut data).unwrap();

        let g: IRawGrammar = match serde_json::from_str(&data) {
            Ok(x) => x,
            Err(err) => {
                println!("error path: {:?}, err: {:?}", grammar_path, err);
                panic!(err);
            }
        };

        Grammar::new(g)
    }

    pub fn for_test(grammar_path: &str) -> Self {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push(grammar_path);

        println!("{:?}", path);

        let mut file = File::open(path).unwrap();
        let mut data = String::new();
        file.read_to_string(&mut data).unwrap();

        let g: IRawGrammar = serde_json::from_str(&data).unwrap();
        Grammar::new(g)
    }

    pub fn from_code(grammar_path: &str, code: &str) -> Self {
        let mut grammar = Grammar::for_test(grammar_path);
        let c_code = String::from(code);
        let mut rule_stack = Some(StackElement::null());
        for line in c_code.lines() {
            let result = grammar.tokenize_line(line, &mut rule_stack);
            rule_stack = result.rule_stack;
            for token in result.tokens {
                let start = token.start_index.clone() as usize;
                let end = token.end_index.clone() as usize;
                let new_line: String = String::from(line)
                    .chars()
                    .skip(start)
                    .take(end - start)
                    .collect();
                let token_str: String = token.scopes.join(", ");
                println!(
                    " - token from {} to {} ({}) with scopes {}",
                    token.start_index, token.end_index, new_line, token_str
                )
            }
        }

        grammar
    }
}

impl IGrammarRegistry for Grammar {
    fn get_external_grammar(
        &self,
        _scope_name: String,
        _repository: IRawRepository,
    ) -> Option<IRawGrammar> {
        None
    }
}

impl IRuleRegistry for Grammar {
    fn register_id(&mut self) -> i32 {
        self.last_rule_id = self.last_rule_id + 1;
        self.last_rule_id.clone()
    }

    fn get_rule(&mut self, pattern_id: i32) -> &mut Box<dyn AbstractRule> {
        self.rule_container.get_rule(pattern_id)
    }

    fn register_rule(&mut self, result: Box<dyn AbstractRule>) -> i32 {
        self.rule_container.register_rule(result)
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;

    use crate::grammar::line_tokens::IToken;
    use crate::grammar::{Grammar, StackElement};
    use crate::rule::abstract_rule::RuleEnum;
    use crate::rule::IRuleRegistry;

    #[test]
    fn should_build_grammar_json() {
        let code = "
#include <stdio.h>
int main() {
printf(\"Hello, World!\");
return 0;
}
";
        let grammar = Grammar::from_code("extensions/cpp/syntaxes/c.tmLanguage.json", code);
        let first_rule = grammar.rule_container.rule_id2desc.get(&1).unwrap();
        assert_eq!(38, first_rule.clone().patterns_length());
        debug_output(&grammar, String::from("program.json"));
    }

    #[test]
    fn should_build_scope_name_map() {
        let code = "
#include <stdio.h>
int main() {
printf(\"Hello, World!\");
return 0;
}
";
        let grammar = Grammar::from_code("extensions/cpp/syntaxes/c.tmLanguage.json", code);
        assert_eq!(172, grammar.scope_name_map.len());
    }

    #[test]
    fn should_identify_c_include() {
        let code = "#include <stdio.h>";
        let mut grammar = Grammar::for_test("extensions/cpp/syntaxes/c.tmLanguage.json");
        let mut rule_stack = Some(StackElement::null());
        let result = grammar.tokenize_line(code, &mut rule_stack);

        assert_eq!(6, result.tokens.len());
        assert_eq!(0, result.tokens[0].start_index);
        assert_eq!(1, result.tokens[1].start_index);
        assert_eq!(8, result.tokens[2].start_index);
        assert_eq!(9, result.tokens[3].start_index);
        assert_eq!(10, result.tokens[4].start_index);
        assert_eq!(17, result.tokens[5].start_index);
    }

    fn debug_output(grammar: &Grammar, path: String) {
        let j = serde_json::to_string(&grammar.rule_container.rule_id2desc).unwrap();
        let mut file = File::create(path).unwrap();
        match file.write_all(j.as_bytes()) {
            Ok(_) => {}
            Err(_) => {}
        };
    }

    #[test]
    fn should_build_json_grammar() {
        let code = "{}";
        let grammar = Grammar::from_code("extensions/json/syntaxes/json.tmLanguage.json", code);
        assert_eq!(grammar.rule_container.rule_id2desc.len(), 35);
        debug_output(&grammar, String::from("program.json"));
    }

    #[test]
    fn should_build_html_grammar_for_back_refs() {
        let code = "<html></html>";
        let grammar = Grammar::from_code("fixtures/test-cases/first-mate/fixtures/html.json", code);
        assert_eq!(grammar.rule_container.rule_id2desc.len(), 101);

        let tokens = get_all_tokens(
            "extensions/html/syntaxes/html.tmLanguage.json",
            code.clone(),
        );
        assert_eq!(1, tokens.len());
    }

    #[test]
    fn should_build_correct_end_rule_id_for_makefile() {
        let code = "CC=gcc
CFLAGS=-I.
DEPS = hellomake.h
OBJ = hellomake.o hellofunc.o
";
        let mut grammar = Grammar::from_code("extensions/make/syntaxes/make.tmLanguage.json", code);
        let mut end_rule_count = 0;
        for (_x, rule) in grammar.rule_container.rule_id2desc.iter() {
            let rule_instance = rule.get_rule_instance();
            if let RuleEnum::BeginEndRule(rule) = rule_instance {
                assert_eq!(rule._end.rule_id, -1);
                end_rule_count = end_rule_count + 1;
            }
        }
        assert_eq!(grammar.get_rule(1).patterns_length(), 6);
        assert_eq!(end_rule_count, 29);
        debug_output(&grammar, String::from("program.json"));
    }

    #[test]
    fn should_build_makefile_grammar() {
        let code = "CC=gcc
CFLAGS=-I.
DEPS = hellomake.h
OBJ = hellomake.o hellofunc.o

%.o: %.c $(DEPS)
\t$(CC) -c -o $@ $< $(CFLAGS)

hellomake: $(OBJ)
\t$(CC) -o $@ $^ $(CFLAGS)";
        let mut grammar = Grammar::from_code("extensions/make/syntaxes/make.tmLanguage.json", code);
        assert_eq!(grammar.rule_container.rule_id2desc.len(), 104);
        assert_eq!(grammar.get_rule(1).patterns_length(), 6);

        let tokens = get_all_tokens(
            "extensions/make/syntaxes/make.tmLanguage.json",
            code.clone(),
        );
        assert_eq!(10, tokens.len());
        let x: Vec<String> = tokens.iter().map(|token| token.len().to_string()).collect();
        assert_eq!(String::from("3,3,4,4,1,9,12,1,6,12"), x.join(","));
    }

    pub fn get_all_tokens(grammar_path: &str, code: &str) -> Vec<Vec<IToken>> {
        let mut grammar = Grammar::for_test(grammar_path);
        let c_code = String::from(code);
        let mut rule_stack = Some(StackElement::null());
        let mut all_tokens: Vec<Vec<IToken>> = vec![];

        for line in c_code.lines() {
            let result = grammar.tokenize_line(line, &mut rule_stack);
            rule_stack = result.rule_stack;
            all_tokens.push(result.tokens);
        }

        all_tokens
    }

    #[test]
    fn should_resolve_make_file_error_issues() {
        let mut grammar = Grammar::for_test("extensions/make/syntaxes/make.tmLanguage.json");
        let result = grammar.tokenize_line("%.o: %.c $(DEPS)", &mut None);
        let tokens = result.tokens.clone();
        assert_eq!(9, tokens.len());
        assert_eq!("source.makefile,meta.scope.target.makefile,entity.name.function.target.makefile,constant.other.placeholder.makefile", tokens[0].scopes.join(","));
        assert_eq!(0, tokens[0].start_index);
        assert_eq!(1, tokens[1].start_index);
        assert_eq!(3, tokens[2].start_index);
        assert_eq!(4, tokens[3].start_index);
        assert_eq!(5, tokens[4].start_index);
        assert_eq!(6, tokens[5].start_index);
        assert_eq!(9, tokens[6].start_index);
        assert_eq!(11, tokens[7].start_index);
        assert_eq!(15, tokens[8].start_index);
        debug_output(&grammar, String::from("program.json"));
    }

    #[test]
    fn should_resolve_make_file_error_issues2() {
        let mut grammar = Grammar::for_test("extensions/make/syntaxes/make.tmLanguage.json");

        let mut rule_stack = Some(StackElement::null());
        let result = grammar.tokenize_line("hellomake: $(OBJ)", &mut rule_stack);
        assert_eq!(6, result.tokens.len());

        rule_stack = result.rule_stack;
        let result2 = grammar.tokenize_line("\t$(CC) -o $@ $^ $(CFLAGS)", &mut rule_stack);
        assert_eq!(12, result2.tokens.len());
    }

    #[test]
    fn should_success_token_for_short_code() {
        let code = "hellomake: $(OBJ)
\t$(CC) -o $@ $^ $(CFLAGS)";
        let tokens = get_all_tokens(
            "fixtures/test-cases/first-mate/fixtures/makefile.json",
            code.clone(),
        );
        assert_eq!(2, tokens.len());
        let x: Vec<String> = tokens.iter().map(|token| token.len().to_string()).collect();
        assert_eq!(String::from("6,14"), x.join(","));
    }

    #[test]
    fn should_build_for_groovy() {
        let mut grammar = Grammar::for_test("extensions/groovy/syntaxes/groovy.tmLanguage.json");
        let result = grammar.tokenize_line("include \":app\"", &mut None);
        let tokens = result.tokens.clone();
        assert_eq!(4, tokens.len());
        assert_eq!(0, tokens[0].start_index);
        assert_eq!(8, tokens[1].start_index);
        assert_eq!(9, tokens[2].start_index);
        assert_eq!(13, tokens[3].start_index);
    }
}
