use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TypeKey(pub(crate) String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct QualifierKey(pub(crate) String);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Key {
    pub(crate) type_key: TypeKey,
    pub(crate) qualifier: Option<QualifierKey>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ProviderId(pub(crate) usize);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RootId(pub(crate) usize);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub(crate) key: Key,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderDescriptor {
    pub(crate) key: Key,
    pub(crate) dependencies: Vec<Request>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RootDescriptor {
    pub(crate) dependencies: Vec<Request>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RequestSlot {
    Root {
        root: RootId,
        dependency_index: usize,
    },
    Provider {
        provider: ProviderId,
        dependency_index: usize,
    },
    Registration {
        provider: ProviderId,
        registration_index: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestBinding {
    pub(crate) slot: RequestSlot,
    pub(crate) provider: ProviderId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GraphPlan {
    pub(crate) construction_order: Vec<ProviderId>,
    pub(crate) bindings: Vec<RequestBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum GraphError {
    DuplicateProvider {
        key: Key,
        provider_ids: Vec<ProviderId>,
    },
    Missing {
        request: Request,
        requester: RequestSlot,
    },
    Ambiguous {
        request: Request,
        candidate_ids: Vec<ProviderId>,
        requester: RequestSlot,
    },
    InvalidQualifier {
        request: Request,
        available_candidates: Vec<ProviderId>,
        requester: RequestSlot,
    },
    Cycle {
        closed_path: Vec<ProviderId>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VisitState {
    Unseen,
    Visiting,
    Resolved,
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    provider: ProviderId,
    next_dependency: usize,
}

pub(crate) struct ProviderIndex {
    exact: HashMap<Key, ProviderId>,
    by_type: HashMap<TypeKey, Vec<ProviderId>>,
}

impl ProviderIndex {
    pub(crate) fn new(providers: &[ProviderDescriptor]) -> Result<Self, GraphError> {
        let mut ids_by_key: HashMap<Key, Vec<ProviderId>> = HashMap::new();
        let mut first_duplicate_key = None;

        for (index, provider) in providers.iter().enumerate() {
            let provider_id = ProviderId(index);
            let ids = ids_by_key.entry(provider.key.clone()).or_default();
            ids.push(provider_id);
            if ids.len() == 2 && first_duplicate_key.is_none() {
                first_duplicate_key = Some(provider.key.clone());
            }
        }

        if let Some(key) = first_duplicate_key {
            let provider_ids = ids_by_key
                .remove(&key)
                .expect("a duplicate key must have provider IDs");
            return Err(GraphError::DuplicateProvider { key, provider_ids });
        }

        let mut exact = HashMap::with_capacity(providers.len());
        let mut by_type: HashMap<TypeKey, Vec<ProviderId>> = HashMap::new();
        for (index, provider) in providers.iter().enumerate() {
            let provider_id = ProviderId(index);
            exact.insert(provider.key.clone(), provider_id);
            by_type
                .entry(provider.key.type_key.clone())
                .or_default()
                .push(provider_id);
        }

        Ok(Self { exact, by_type })
    }

    pub(crate) fn resolve_request(
        &self,
        request: &Request,
        requester: RequestSlot,
    ) -> Result<ProviderId, GraphError> {
        let candidates = self.by_type.get(&request.key.type_key);

        if request.key.qualifier.is_some() {
            if let Some(provider_id) = self.exact.get(&request.key) {
                return Ok(*provider_id);
            }

            return match candidates {
                None => Err(GraphError::Missing {
                    request: request.clone(),
                    requester,
                }),
                Some(available_candidates) => Err(GraphError::InvalidQualifier {
                    request: request.clone(),
                    available_candidates: available_candidates.clone(),
                    requester,
                }),
            };
        }

        match candidates {
            None => Err(GraphError::Missing {
                request: request.clone(),
                requester,
            }),
            Some(candidates) if candidates.len() == 1 => Ok(candidates[0]),
            Some(candidates) => Err(GraphError::Ambiguous {
                request: request.clone(),
                candidate_ids: candidates.clone(),
                requester,
            }),
        }
    }
}

pub(crate) fn resolve(
    providers: &[ProviderDescriptor],
    roots: &[RootDescriptor],
) -> Result<GraphPlan, GraphError> {
    // Duplicate validation is global and precedes the empty-root fast path.
    let index = ProviderIndex::new(providers)?;

    if roots.iter().all(|root| root.dependencies.is_empty()) {
        return Ok(GraphPlan {
            construction_order: Vec::new(),
            bindings: Vec::new(),
        });
    }

    let mut states = vec![VisitState::Unseen; providers.len()];
    let mut bindings = Vec::new();
    let mut stack = Vec::new();

    for (root_index, root) in roots.iter().enumerate() {
        for (dependency_index, request) in root.dependencies.iter().enumerate() {
            let slot = RequestSlot::Root {
                root: RootId(root_index),
                dependency_index,
            };
            let provider_id = index.resolve_request(request, slot)?;
            bindings.push(RequestBinding {
                slot,
                provider: provider_id,
            });

            if states[provider_id.0] == VisitState::Unseen {
                states[provider_id.0] = VisitState::Visiting;
                stack.push(Frame {
                    provider: provider_id,
                    next_dependency: 0,
                });
                resolve_depth_first(providers, &index, &mut states, &mut bindings, &mut stack)?;
            }
        }
    }

    let construction_order = topological_order(providers, &states, &bindings);
    Ok(GraphPlan {
        construction_order,
        bindings,
    })
}

fn resolve_depth_first(
    providers: &[ProviderDescriptor],
    index: &ProviderIndex,
    states: &mut [VisitState],
    bindings: &mut Vec<RequestBinding>,
    stack: &mut Vec<Frame>,
) -> Result<(), GraphError> {
    while let Some(frame) = stack.last_mut() {
        let provider_id = frame.provider;
        let dependencies = &providers[provider_id.0].dependencies;

        if frame.next_dependency == dependencies.len() {
            states[provider_id.0] = VisitState::Resolved;
            stack.pop();
            continue;
        }

        let dependency_index = frame.next_dependency;
        frame.next_dependency += 1;
        let request = &dependencies[dependency_index];
        let slot = RequestSlot::Provider {
            provider: provider_id,
            dependency_index,
        };
        let dependency_id = index.resolve_request(request, slot)?;
        bindings.push(RequestBinding {
            slot,
            provider: dependency_id,
        });

        match states[dependency_id.0] {
            VisitState::Unseen => {
                states[dependency_id.0] = VisitState::Visiting;
                stack.push(Frame {
                    provider: dependency_id,
                    next_dependency: 0,
                });
            }
            VisitState::Visiting => {
                let cycle_start = stack
                    .iter()
                    .position(|frame| frame.provider == dependency_id)
                    .expect("visiting providers must be in the traversal stack");
                let mut closed_path: Vec<_> = stack[cycle_start..]
                    .iter()
                    .map(|frame| frame.provider)
                    .collect();
                closed_path.push(dependency_id);
                return Err(GraphError::Cycle { closed_path });
            }
            VisitState::Resolved => {}
        }
    }

    Ok(())
}

fn topological_order(
    providers: &[ProviderDescriptor],
    states: &[VisitState],
    bindings: &[RequestBinding],
) -> Vec<ProviderId> {
    let mut dependency_bindings: Vec<Vec<Option<ProviderId>>> = providers
        .iter()
        .map(|provider| vec![None; provider.dependencies.len()])
        .collect();

    for binding in bindings {
        if let RequestSlot::Provider {
            provider,
            dependency_index,
        } = binding.slot
        {
            dependency_bindings[provider.0][dependency_index] = Some(binding.provider);
        }
    }

    let mut indegree = vec![0usize; providers.len()];
    let mut dependents = vec![Vec::new(); providers.len()];

    for (provider_index, provider_dependencies) in dependency_bindings.iter().enumerate() {
        if states[provider_index] == VisitState::Unseen {
            continue;
        }

        let provider_id = ProviderId(provider_index);
        let mut unique_dependencies = HashSet::new();
        for dependency_binding in provider_dependencies {
            let dependency_id =
                dependency_binding.expect("reachable provider dependencies must have bindings");
            if unique_dependencies.insert(dependency_id) {
                indegree[provider_index] += 1;
                dependents[dependency_id.0].push(provider_id);
            }
        }
    }

    let mut ready = BinaryHeap::new();
    for (provider_index, state) in states.iter().enumerate() {
        if *state != VisitState::Unseen && indegree[provider_index] == 0 {
            ready.push(Reverse(provider_index));
        }
    }

    let mut order = Vec::new();
    while let Some(Reverse(provider_index)) = ready.pop() {
        order.push(ProviderId(provider_index));
        for dependent in &dependents[provider_index] {
            indegree[dependent.0] -= 1;
            if indegree[dependent.0] == 0 {
                ready.push(Reverse(dependent.0));
            }
        }
    }

    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(type_key: &str, qualifier: Option<&str>) -> Key {
        Key {
            type_key: TypeKey(type_key.to_owned()),
            qualifier: qualifier.map(|value| QualifierKey(value.to_owned())),
        }
    }

    fn request(type_key: &str, qualifier: Option<&str>) -> Request {
        Request {
            key: key(type_key, qualifier),
        }
    }

    fn provider(
        type_key: &str,
        qualifier: Option<&str>,
        dependencies: &[(&str, Option<&str>)],
    ) -> ProviderDescriptor {
        ProviderDescriptor {
            key: key(type_key, qualifier),
            dependencies: dependencies
                .iter()
                .map(|(dependency_type, qualifier)| request(dependency_type, *qualifier))
                .collect(),
        }
    }

    fn root(dependencies: &[(&str, Option<&str>)]) -> RootDescriptor {
        RootDescriptor {
            dependencies: dependencies
                .iter()
                .map(|(dependency_type, qualifier)| request(dependency_type, *qualifier))
                .collect(),
        }
    }

    fn resolve_ok(providers: &[ProviderDescriptor], roots: &[RootDescriptor]) -> GraphPlan {
        resolve(providers, roots).expect("graph should resolve")
    }

    #[test]
    fn resolves_config_database_service_chain_dependency_first() {
        let providers = [
            provider("Config", None, &[]),
            provider("Database", None, &[("Config", None)]),
            provider("Service", None, &[("Database", None)]),
        ];
        let roots = [root(&[("Service", None)])];

        let plan = resolve_ok(&providers, &roots);

        assert_eq!(
            plan.construction_order,
            vec![ProviderId(0), ProviderId(1), ProviderId(2)]
        );
        assert_eq!(
            plan.bindings,
            vec![
                RequestBinding {
                    slot: RequestSlot::Root {
                        root: RootId(0),
                        dependency_index: 0,
                    },
                    provider: ProviderId(2),
                },
                RequestBinding {
                    slot: RequestSlot::Provider {
                        provider: ProviderId(2),
                        dependency_index: 0,
                    },
                    provider: ProviderId(1),
                },
                RequestBinding {
                    slot: RequestSlot::Provider {
                        provider: ProviderId(1),
                        dependency_index: 0,
                    },
                    provider: ProviderId(0),
                },
            ]
        );
    }

    #[test]
    fn keeps_each_root_and_provider_request_binding_but_resolves_provider_once() {
        let providers = [
            provider("Shared", None, &[]),
            provider("Consumer", None, &[("Shared", None), ("Shared", None)]),
        ];
        let roots = [root(&[("Consumer", None)]), root(&[("Shared", None)])];

        let plan = resolve_ok(&providers, &roots);

        assert_eq!(plan.construction_order, vec![ProviderId(0), ProviderId(1)]);
        assert_eq!(plan.bindings.len(), 4);
        assert_eq!(
            plan.bindings
                .iter()
                .filter(|binding| binding.provider == ProviderId(0))
                .count(),
            3
        );
        assert!(plan.bindings.contains(&RequestBinding {
            slot: RequestSlot::Provider {
                provider: ProviderId(1),
                dependency_index: 0,
            },
            provider: ProviderId(0),
        }));
        assert!(plan.bindings.contains(&RequestBinding {
            slot: RequestSlot::Provider {
                provider: ProviderId(1),
                dependency_index: 1,
            },
            provider: ProviderId(0),
        }));
        assert!(plan.bindings.contains(&RequestBinding {
            slot: RequestSlot::Root {
                root: RootId(1),
                dependency_index: 0,
            },
            provider: ProviderId(0),
        }));
    }

    #[test]
    fn omits_unreachable_providers_and_ignores_their_broken_dependencies() {
        let providers = [
            provider("Reachable", None, &[]),
            provider("Unreachable", None, &[("Missing", None)]),
        ];
        let roots = [root(&[("Reachable", None)])];

        let plan = resolve_ok(&providers, &roots);

        assert_eq!(plan.construction_order, vec![ProviderId(0)]);
        assert_eq!(plan.bindings.len(), 1);
    }

    #[test]
    fn reports_first_global_duplicate_even_when_unreachable_or_roots_are_empty() {
        let providers = [
            provider("Unused", None, &[]),
            provider("Used", None, &[]),
            provider("Unused", None, &[]),
            provider("Unused", None, &[]),
        ];

        let error = resolve(&providers, &[]).unwrap_err();

        assert_eq!(
            error,
            GraphError::DuplicateProvider {
                key: key("Unused", None),
                provider_ids: vec![ProviderId(0), ProviderId(2), ProviderId(3)],
            }
        );
    }

    #[test]
    fn duplicate_group_precedence_uses_first_second_occurrence() {
        let providers = [
            provider("A", None, &[]),
            provider("B", None, &[]),
            provider("B", None, &[]),
            provider("A", None, &[]),
        ];

        assert_eq!(
            resolve(&providers, &[]).unwrap_err(),
            GraphError::DuplicateProvider {
                key: key("B", None),
                provider_ids: vec![ProviderId(1), ProviderId(2)],
            }
        );
    }

    #[test]
    fn duplicate_dependency_edges_do_not_block_topological_order() {
        let providers = [
            provider("Dependency", None, &[]),
            provider(
                "Consumer",
                None,
                &[("Dependency", None), ("Dependency", None)],
            ),
        ];
        let roots = [root(&[("Consumer", None)])];

        let plan = resolve_ok(&providers, &roots);

        assert_eq!(plan.construction_order, vec![ProviderId(0), ProviderId(1)]);
    }

    #[test]
    fn matches_qualified_requests_exactly_and_allows_unique_qualified_unqualified_match() {
        let providers = [
            provider("Database", Some("Primary"), &[]),
            provider("Other", None, &[("Database", Some("Primary"))]),
            provider("UsesUnique", None, &[("Database", None)]),
        ];
        let roots = [root(&[("Other", None), ("UsesUnique", None)])];

        let plan = resolve_ok(&providers, &roots);

        assert_eq!(
            plan.construction_order,
            vec![ProviderId(0), ProviderId(1), ProviderId(2)]
        );
        assert!(plan.bindings.contains(&RequestBinding {
            slot: RequestSlot::Provider {
                provider: ProviderId(2),
                dependency_index: 0,
            },
            provider: ProviderId(0),
        }));
    }

    #[test]
    fn reports_ambiguous_candidates_in_declaration_order() {
        let providers = [
            provider("Database", Some("Primary"), &[]),
            provider("Database", Some("Replica"), &[]),
        ];
        let roots = [root(&[("Database", None)])];

        assert_eq!(
            resolve(&providers, &roots).unwrap_err(),
            GraphError::Ambiguous {
                request: request("Database", None),
                candidate_ids: vec![ProviderId(0), ProviderId(1)],
                requester: RequestSlot::Root {
                    root: RootId(0),
                    dependency_index: 0,
                },
            }
        );
    }

    #[test]
    fn distinguishes_invalid_qualifier_from_missing_type() {
        let providers = [provider("Database", Some("Primary"), &[])];
        let invalid_qualifier_roots = [root(&[("Database", Some("Replica"))])];
        let missing_roots = [root(&[("Config", Some("Primary"))])];

        assert_eq!(
            resolve(&providers, &invalid_qualifier_roots).unwrap_err(),
            GraphError::InvalidQualifier {
                request: request("Database", Some("Replica")),
                available_candidates: vec![ProviderId(0)],
                requester: RequestSlot::Root {
                    root: RootId(0),
                    dependency_index: 0,
                },
            }
        );
        assert_eq!(
            resolve(&providers, &missing_roots).unwrap_err(),
            GraphError::Missing {
                request: request("Config", Some("Primary")),
                requester: RequestSlot::Root {
                    root: RootId(0),
                    dependency_index: 0,
                },
            }
        );
    }

    #[test]
    fn reports_missing_for_unqualified_request_without_provider() {
        let roots = [root(&[("Missing", None)])];

        assert_eq!(
            resolve(&[], &roots).unwrap_err(),
            GraphError::Missing {
                request: request("Missing", None),
                requester: RequestSlot::Root {
                    root: RootId(0),
                    dependency_index: 0,
                },
            }
        );
    }

    #[test]
    fn reports_first_resolution_error_in_depth_first_order() {
        let providers = [
            provider("First", None, &[("NestedMissing", None)]),
            provider("Second", None, &[]),
        ];
        let roots = [root(&[("First", None), ("RootMissing", None)])];

        assert_eq!(
            resolve(&providers, &roots).unwrap_err(),
            GraphError::Missing {
                request: request("NestedMissing", None),
                requester: RequestSlot::Provider {
                    provider: ProviderId(0),
                    dependency_index: 0,
                },
            }
        );
    }

    #[test]
    fn reports_a_closed_cycle_path() {
        let providers = [
            provider("A", None, &[("B", None)]),
            provider("B", None, &[("C", None)]),
            provider("C", None, &[("A", None)]),
        ];
        let roots = [root(&[("A", None)])];

        assert_eq!(
            resolve(&providers, &roots).unwrap_err(),
            GraphError::Cycle {
                closed_path: vec![ProviderId(0), ProviderId(1), ProviderId(2), ProviderId(0)],
            }
        );
    }

    #[test]
    fn empty_roots_or_empty_root_dependencies_produce_empty_plan() {
        let providers = [provider("Unused", None, &[])];

        assert_eq!(
            resolve_ok(&providers, &[]),
            GraphPlan {
                construction_order: Vec::new(),
                bindings: Vec::new(),
            }
        );
        assert_eq!(
            resolve_ok(&providers, &[root(&[])]),
            GraphPlan {
                construction_order: Vec::new(),
                bindings: Vec::new(),
            }
        );
    }

    #[test]
    fn declaration_index_breaks_topological_ties_and_results_are_repeatable() {
        let providers = [
            provider("FirstIndependent", None, &[]),
            provider("SecondIndependent", None, &[]),
            provider(
                "Join",
                None,
                &[("SecondIndependent", None), ("FirstIndependent", None)],
            ),
        ];
        let roots = [root(&[("Join", None)])];

        let plan = resolve_ok(&providers, &roots);
        assert_eq!(
            plan.construction_order,
            vec![ProviderId(0), ProviderId(1), ProviderId(2)]
        );
        assert_eq!(resolve_ok(&providers, &roots), plan);

        let permuted_providers = [
            provider("SecondIndependent", None, &[]),
            provider("FirstIndependent", None, &[]),
            provider(
                "Join",
                None,
                &[("FirstIndependent", None), ("SecondIndependent", None)],
            ),
        ];
        let permuted_plan = resolve_ok(&permuted_providers, &roots);
        assert_eq!(
            permuted_plan.construction_order,
            vec![ProviderId(0), ProviderId(1), ProviderId(2)]
        );
        assert_eq!(resolve_ok(&permuted_providers, &roots), permuted_plan);
    }

    #[test]
    fn resolves_a_deep_chain_iteratively() {
        const DEPTH: usize = 20_000;

        let mut providers = Vec::with_capacity(DEPTH);
        providers.push(provider("Type0", None, &[]));
        for index in 1..DEPTH {
            let dependency = format!("Type{}", index - 1);
            let output = format!("Type{index}");
            providers.push(provider(&output, None, &[(dependency.as_str(), None)]));
        }
        let root_type = format!("Type{}", DEPTH - 1);
        let roots = [root(&[(root_type.as_str(), None)])];

        let plan = resolve_ok(&providers, &roots);

        assert_eq!(plan.construction_order.len(), DEPTH);
        assert_eq!(plan.construction_order.first(), Some(&ProviderId(0)));
        assert_eq!(plan.construction_order.last(), Some(&ProviderId(DEPTH - 1)));
    }
}
