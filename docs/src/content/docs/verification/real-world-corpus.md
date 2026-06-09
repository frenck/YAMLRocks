---
title: Real-world verification
description: The public YAML configuration corpus YAMLRocks continuously tests against.
---

YAMLRocks is tested against YAML that people actually write, review, and keep in
version control. The real-world corpus pulls public configuration repositories in
as pinned git submodules and runs them through the same parser and round-trip
emitter users get from the Python package.

The promise is deliberately narrow and testable: every standalone YAML file in
the corpus must parse and re-emit byte-for-byte in `OPT_ROUND_TRIP` mode. For
Home Assistant, selected repositories are also loaded through `configuration.yaml`
with native `!include` resolution enabled, then checked that every unmodified
source file writes back byte-for-byte.

:::note[Compatibility corpus, not endorsement]
The projects listed here do not endorse, depend on, or test YAMLRocks themselves.
Their public repositories are used as a reproducible compatibility corpus so
regressions are caught against real configuration shapes.
:::

## Current corpus

The corpus currently spans **95 public repositories across 25 ecosystems** and
roughly **22,700 YAML files**. Each submodule is pinned to a specific commit by
this repository, so failures are reproducible until the corpus is deliberately
refreshed. The `Files` column counts `*.yaml` and `*.yml` files after excluding
Helm chart templates.

| Ecosystem       | Repos | Files | Sources                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| --------------- | ----: | ----: | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Home Assistant  |    15 |  1756 | [Bahnburner/Home-Assistant-Config][ha-bahnburner], [DubhAd/Home-AssistantConfig][ha-dubhad], [arsaboo/homeassistant-config][ha-arsaboo], [bachya/smart-home][ha-bachya], [basnijholt/home-assistant-config][ha-basnijholt], [benct/home-assistant-config][ha-benct], [bieniu/home-assistant-config][ha-bieniu], [dshokouhi/Home-AssistantConfig][ha-dshokouhi], [frenck/home-assistant-config][ha-frenck], [hmmbob/HomeAssistantConfig][ha-hmmbob], [jcallaghan/home-assistant-config][ha-jcallaghan], [nagyrobi/home-assistant-configuration-examples][ha-nagyrobi], [renemarc/home-assistant-config][ha-renemarc], [shortbloke/home_assistant_config][ha-shortbloke], [thomasloven/hass-config][ha-thomasloven] |
| Ansible         |     7 |   674 | [dev-sec/ansible-collection-hardening][ansible-dev-sec], [geerlingguy/ansible-for-devops][ansible-devops], [geerlingguy/ansible-role-docker][ansible-docker], [geerlingguy/ansible-role-mysql][ansible-mysql], [geerlingguy/ansible-role-nginx][ansible-nginx], [geerlingguy/mac-dev-playbook][ansible-mac], [prometheus-community/ansible][ansible-prometheus]                                                                                                                                                                                                                                                                                                                                                   |
| ESPHome         |     7 |  3264 | [AlexMekkering/esphome-config][esphome-alexmekkering], [athom-tech/esp32-configs][esphome-athom], [esphome/esphome][esphome-core], [esphome/firmware][esphome-firmware], [jesserockz/esphome-configs][esphome-jesserockz], [landonr/lilygo-tdisplays3-esphome][esphome-landonr], [nrandell/esphome][esphome-nrandell]                                                                                                                                                                                                                                                                                                                                                                                             |
| Kubernetes      |     6 |  1175 | [GoogleCloudPlatform/microservices-demo][kubernetes-microservices], [dockersamples/example-voting-app][kubernetes-voting], [kelseyhightower/kubernetes-the-hard-way][kubernetes-hard-way], [kubernetes-sigs/kubespray][kubernetes-kubespray], [kubernetes-sigs/kustomize][kubernetes-kustomize], [kubernetes/examples][kubernetes-examples]                                                                                                                                                                                                                                                                                                                                                                       |
| Docker Compose  |     5 |   493 | [Haxxnet/Compose-Examples][compose-haxxnet], [compose-spec/compose-spec][compose-spec], [docker/awesome-compose][compose-awesome], [docker/compose][compose-tool], [vegasbrianc/prometheus][compose-vegasbrianc]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| GitOps          |     5 |   569 | [argoproj/argocd-example-apps][gitops-argocd], [fluxcd/flux2][gitops-flux2], [fluxcd/flux2-kustomize-helm-example][gitops-flux], [rancher/fleet-examples][gitops-fleet], [stefanprodan/podinfo][gitops-podinfo]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| CircleCI        |     4 |    91 | [CircleCI-Public/circleci-demo-go][circleci-go], [CircleCI-Public/circleci-demo-javascript-express][circleci-js], [CircleCI-Public/circleci-demo-python-django][circleci-python], [circleci/circleci-docs][circleci-docs]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Helm            |     4 |  2002 | [grafana/helm-charts][helm-grafana], [helm/helm][helm-tool], [jenkinsci/helm-charts][helm-jenkins], [prometheus-community/helm-charts][helm-prometheus]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| OpenAPI         |     4 |   112 | [OAI/OpenAPI-Specification][openapi-spec], [readmeio/oas-examples][openapi-oas-examples], [stripe/openapi][openapi-stripe], [swagger-api/swagger-petstore][openapi-petstore]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Prometheus      |     4 |   713 | [prometheus-operator/kube-prometheus][prometheus-kube], [prometheus-operator/prometheus-operator][prometheus-operator], [prometheus/alertmanager][prometheus-alertmanager], [prometheus/prometheus][prometheus-prometheus]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Argo            |     3 |  3398 | [argoproj/argo-cd][argo-cd], [argoproj/argo-rollouts][argo-rollouts], [argoproj/argo-workflows][argo-workflows]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| CloudFormation  |     3 |   397 | [aws-cloudformation/aws-cloudformation-templates][cloudformation-aws], [awslabs/aws-cloudformation-templates][cloudformation-awslabs], [widdix/aws-cf-templates][cloudformation-widdix]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| dbt             |     3 |   134 | [dbt-labs/dbt-core][dbt-core], [dbt-labs/dbt-utils][dbt-utils], [dbt-labs/jaffle-shop-classic][dbt-jaffle-shop]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| GitHub Actions  |     3 |   277 | [actions/setup-node][github-actions-setup-node], [actions/starter-workflows][github-actions-starter], [actions/toolkit][github-actions-toolkit]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| OpenTelemetry   |     3 |  3526 | [open-telemetry/opentelemetry-collector][opentelemetry-collector], [open-telemetry/opentelemetry-collector-contrib][opentelemetry-contrib], [open-telemetry/opentelemetry-demo][opentelemetry-demo]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Tekton          |     3 |  1787 | [tektoncd/catalog][tekton-catalog], [tektoncd/pipeline][tekton-pipeline], [tektoncd/triggers][tekton-triggers]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Azure Pipelines |     2 |    50 | [MicrosoftDocs/pipelines-java][azure-pipelines-java], [microsoft/azure-pipelines-yaml][azure-pipelines-yaml]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Cloud Foundry   |     2 |   354 | [cloudfoundry/bosh-deployment][cloudfoundry-bosh], [cloudfoundry/cf-deployment][cloudfoundry-cf]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Concourse       |     2 |   234 | [concourse/concourse][concourse-concourse], [concourse/concourse-docker][concourse-docker]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Crossplane      |     2 |   371 | [crossplane/crossplane][crossplane-crossplane], [upbound/platform-ref-aws][crossplane-aws]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| MkDocs          |     2 |    31 | [mkdocstrings/mkdocstrings][mkdocs-mkdocstrings], [squidfunk/mkdocs-material][mkdocs-material]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Serverless      |     2 |   816 | [aws-samples/serverless-patterns][serverless-patterns], [serverless/examples][serverless-examples]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Woodpecker      |     2 |   130 | [woodpecker-ci/plugin-git][woodpecker-plugin-git], [woodpecker-ci/woodpecker][woodpecker-woodpecker]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| cloud-init      |     1 |   262 | [canonical/cloud-init][cloud-init]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| goss            |     1 |    95 | [goss-org/goss][goss]                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |

See the test-suite notes in
[`tests/realworld/README.md`](https://github.com/frenck/yamlrocks/blob/main/tests/realworld/README.md)
for the exact layout and update workflow.

[ansible-dev-sec]: https://github.com/dev-sec/ansible-collection-hardening
[ansible-devops]: https://github.com/geerlingguy/ansible-for-devops
[ansible-docker]: https://github.com/geerlingguy/ansible-role-docker
[ansible-mac]: https://github.com/geerlingguy/mac-dev-playbook
[ansible-mysql]: https://github.com/geerlingguy/ansible-role-mysql
[ansible-nginx]: https://github.com/geerlingguy/ansible-role-nginx
[ansible-prometheus]: https://github.com/prometheus-community/ansible
[argo-cd]: https://github.com/argoproj/argo-cd
[argo-rollouts]: https://github.com/argoproj/argo-rollouts
[argo-workflows]: https://github.com/argoproj/argo-workflows
[azure-pipelines-java]: https://github.com/MicrosoftDocs/pipelines-java
[azure-pipelines-yaml]: https://github.com/microsoft/azure-pipelines-yaml
[circleci-go]: https://github.com/CircleCI-Public/circleci-demo-go
[circleci-js]: https://github.com/CircleCI-Public/circleci-demo-javascript-express
[circleci-python]: https://github.com/CircleCI-Public/circleci-demo-python-django
[circleci-docs]: https://github.com/circleci/circleci-docs
[cloud-init]: https://github.com/canonical/cloud-init
[cloudformation-aws]: https://github.com/aws-cloudformation/aws-cloudformation-templates
[cloudformation-awslabs]: https://github.com/awslabs/aws-cloudformation-templates
[cloudformation-widdix]: https://github.com/widdix/aws-cf-templates
[cloudfoundry-bosh]: https://github.com/cloudfoundry/bosh-deployment
[cloudfoundry-cf]: https://github.com/cloudfoundry/cf-deployment
[compose-awesome]: https://github.com/docker/awesome-compose
[compose-haxxnet]: https://github.com/Haxxnet/Compose-Examples
[compose-spec]: https://github.com/compose-spec/compose-spec
[compose-tool]: https://github.com/docker/compose
[compose-vegasbrianc]: https://github.com/vegasbrianc/prometheus
[concourse-concourse]: https://github.com/concourse/concourse
[concourse-docker]: https://github.com/concourse/concourse-docker
[crossplane-aws]: https://github.com/upbound/platform-ref-aws
[crossplane-crossplane]: https://github.com/crossplane/crossplane
[dbt-core]: https://github.com/dbt-labs/dbt-core
[dbt-jaffle-shop]: https://github.com/dbt-labs/jaffle-shop-classic
[dbt-utils]: https://github.com/dbt-labs/dbt-utils
[esphome-alexmekkering]: https://github.com/AlexMekkering/esphome-config
[esphome-athom]: https://github.com/athom-tech/esp32-configs
[esphome-core]: https://github.com/esphome/esphome
[esphome-firmware]: https://github.com/esphome/firmware
[esphome-jesserockz]: https://github.com/jesserockz/esphome-configs
[esphome-landonr]: https://github.com/landonr/lilygo-tdisplays3-esphome
[esphome-nrandell]: https://github.com/nrandell/esphome
[github-actions-starter]: https://github.com/actions/starter-workflows
[github-actions-setup-node]: https://github.com/actions/setup-node
[github-actions-toolkit]: https://github.com/actions/toolkit
[gitops-argocd]: https://github.com/argoproj/argocd-example-apps
[gitops-fleet]: https://github.com/rancher/fleet-examples
[gitops-flux]: https://github.com/fluxcd/flux2-kustomize-helm-example
[gitops-flux2]: https://github.com/fluxcd/flux2
[gitops-podinfo]: https://github.com/stefanprodan/podinfo
[goss]: https://github.com/goss-org/goss
[ha-arsaboo]: https://github.com/arsaboo/homeassistant-config
[ha-bachya]: https://github.com/bachya/smart-home
[ha-bahnburner]: https://github.com/Bahnburner/Home-Assistant-Config
[ha-basnijholt]: https://github.com/basnijholt/home-assistant-config
[ha-benct]: https://github.com/benct/home-assistant-config
[ha-bieniu]: https://github.com/bieniu/home-assistant-config
[ha-dshokouhi]: https://github.com/dshokouhi/Home-AssistantConfig
[ha-dubhad]: https://github.com/DubhAd/Home-AssistantConfig
[ha-frenck]: https://github.com/frenck/home-assistant-config
[ha-hmmbob]: https://github.com/hmmbob/HomeAssistantConfig
[ha-jcallaghan]: https://github.com/jcallaghan/home-assistant-config
[ha-nagyrobi]: https://github.com/nagyrobi/home-assistant-configuration-examples
[ha-renemarc]: https://github.com/renemarc/home-assistant-config
[ha-shortbloke]: https://github.com/shortbloke/home_assistant_config
[ha-thomasloven]: https://github.com/thomasloven/hass-config
[helm-grafana]: https://github.com/grafana/helm-charts
[helm-jenkins]: https://github.com/jenkinsci/helm-charts
[helm-prometheus]: https://github.com/prometheus-community/helm-charts
[helm-tool]: https://github.com/helm/helm
[kubernetes-examples]: https://github.com/kubernetes/examples
[kubernetes-hard-way]: https://github.com/kelseyhightower/kubernetes-the-hard-way
[kubernetes-kubespray]: https://github.com/kubernetes-sigs/kubespray
[kubernetes-kustomize]: https://github.com/kubernetes-sigs/kustomize
[kubernetes-microservices]: https://github.com/GoogleCloudPlatform/microservices-demo
[kubernetes-voting]: https://github.com/dockersamples/example-voting-app
[mkdocs-material]: https://github.com/squidfunk/mkdocs-material
[mkdocs-mkdocstrings]: https://github.com/mkdocstrings/mkdocstrings
[openapi-oas-examples]: https://github.com/readmeio/oas-examples
[openapi-petstore]: https://github.com/swagger-api/swagger-petstore
[openapi-spec]: https://github.com/OAI/OpenAPI-Specification
[openapi-stripe]: https://github.com/stripe/openapi
[opentelemetry-collector]: https://github.com/open-telemetry/opentelemetry-collector
[opentelemetry-contrib]: https://github.com/open-telemetry/opentelemetry-collector-contrib
[opentelemetry-demo]: https://github.com/open-telemetry/opentelemetry-demo
[prometheus-alertmanager]: https://github.com/prometheus/alertmanager
[prometheus-kube]: https://github.com/prometheus-operator/kube-prometheus
[prometheus-operator]: https://github.com/prometheus-operator/prometheus-operator
[prometheus-prometheus]: https://github.com/prometheus/prometheus
[serverless-examples]: https://github.com/serverless/examples
[serverless-patterns]: https://github.com/aws-samples/serverless-patterns
[tekton-catalog]: https://github.com/tektoncd/catalog
[tekton-pipeline]: https://github.com/tektoncd/pipeline
[tekton-triggers]: https://github.com/tektoncd/triggers
[woodpecker-plugin-git]: https://github.com/woodpecker-ci/plugin-git
[woodpecker-woodpecker]: https://github.com/woodpecker-ci/woodpecker

## What is verified

### Per-file parse and round-trip

Every `*.yaml` and `*.yml` file discovered in the checked-out corpus is parsed in
round-trip mode and immediately emitted again. The emitted bytes must exactly
match the original file.

Custom tags such as `!include`, `!secret`, `!vault`, and other application tags
are preserved in this mode rather than resolved, so the test checks whether the
file is valid YAML and whether YAMLRocks can keep its comments, anchors, scalar
styles, layout, and tags intact.

### Home Assistant include graphs

Home Assistant configurations get an additional test because split
configuration is one of YAMLRocks's core use cases. Repositories with a
`configuration.yaml` are loaded with `OPT_INCLUDES | OPT_ROUND_TRIP`, which
resolves their `!include` tree. The test then verifies two write-back properties:

- the root document re-emits with its include directives restored;
- every resolved source file re-emits byte-for-byte when it was not modified.

Some public Home Assistant repositories reference files that were intentionally
not committed, such as secrets or generated credentials. Those include graphs are
marked as strict expected failures, so they cannot hide a parser regression.

## Scope by ecosystem

The corpus proves that YAMLRocks handles the YAML shapes in these public
repositories. It does not claim to replace each ecosystem's own validation,
rendering, or runtime semantics.

| Ecosystem       | Verified                                                                                | Not claimed                                                                                            |
| --------------- | --------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Home Assistant  | Standalone files round-trip; selected include graphs load and write back byte-for-byte. | Validation of Home Assistant integrations or unavailable private secrets.                              |
| ESPHome         | Device configuration YAML parses and preserves application tags.                        | Execution of ESPHome substitutions, code generation, or `!lambda` semantics.                           |
| Ansible         | Playbooks, roles, inventories, and custom tags such as `!vault` are preserved.          | Replacement of Ansible's loader, inventory plugins, or task execution semantics.                       |
| Kubernetes      | Public manifests and examples parse and round-trip byte-for-byte.                       | Kubernetes API schema validation or `kubectl` behavior.                                                |
| Docker Compose  | Compose examples parse and preserve layout.                                             | Compose model validation or container runtime behavior.                                                |
| CloudFormation  | Template YAML parses and round-trips.                                                   | AWS resource validation or CloudFormation intrinsic execution.                                         |
| GitOps          | Argo CD, Flux, Fleet, and related GitOps examples parse and round-trip.                 | Controller reconciliation or rendered cluster state.                                                   |
| Helm            | Non-template YAML in charts and examples parses and round-trips.                        | Raw Go template files under chart `templates/`, which are not standalone YAML until Helm renders them. |
| OpenAPI         | Specification and example YAML parses and round-trips.                                  | OpenAPI semantic validation.                                                                           |
| dbt             | Project and package YAML parses and round-trips.                                        | dbt model compilation or project validation.                                                           |
| CircleCI        | Pipeline configuration YAML parses and round-trips.                                     | CircleCI configuration validation or job execution.                                                    |
| GitHub Actions  | Workflow examples parse and round-trip.                                                 | GitHub Actions workflow validation or runner behavior.                                                 |
| Serverless      | Framework examples parse and round-trip.                                                | Provider-specific deployment validation.                                                               |
| Tekton          | Task and pipeline YAML parses and round-trips.                                          | Kubernetes admission or Tekton controller behavior.                                                    |
| Prometheus      | Prometheus, Alertmanager, and operator configuration YAML parses and round-trips.       | Prometheus rule validation, query validation, or operator behavior.                                    |
| Argo            | Argo CD, Argo Workflows, and Argo Rollouts YAML parses and round-trips.                 | Argo controller behavior or workflow execution.                                                        |
| OpenTelemetry   | Collector, contrib, and demo configuration YAML parses and round-trips.                 | Collector component validation or telemetry processing behavior.                                       |
| Azure Pipelines | Pipeline example YAML parses and round-trips.                                           | Azure DevOps pipeline validation or job execution.                                                     |
| Cloud Foundry   | BOSH and Cloud Foundry deployment YAML parses and round-trips.                          | BOSH deployment semantics or Cloud Foundry runtime behavior.                                           |
| Concourse       | Concourse pipeline and deployment YAML parses and round-trips.                          | Concourse pipeline validation or worker behavior.                                                      |
| Crossplane      | Crossplane package and platform YAML parses and round-trips.                            | Crossplane schema validation, composition rendering, or controller behavior.                           |
| MkDocs          | MkDocs project configuration YAML parses and round-trips.                               | MkDocs plugin loading or site build behavior.                                                          |
| Woodpecker      | Woodpecker pipeline and plugin YAML parses and round-trips.                             | Woodpecker CI validation or job execution.                                                             |
| cloud-init      | cloud-init YAML examples parse and round-trip.                                          | cloud-init module validation or boot-time behavior.                                                    |
| goss            | goss YAML tests parse and round-trip.                                                   | goss assertion execution.                                                                              |

## Reproduce it locally

Fetch the corpus once, then run the real-world category:

```sh
git submodule update --init
uv run pytest tests/realworld -m realworld
```

Run a single ecosystem by filtering the test ids:

```sh
uv run pytest tests/realworld -k ansible
uv run pytest tests/realworld -k kubernetes
```

The category auto-skips when the submodules are not checked out, so everyday
contributors can still run the normal test suite without downloading the full
corpus.

## Known invalid files

A small number of third-party files use a `.yaml` or `.yml` extension but are not
valid standalone YAML, usually because they are templates that another tool must
render first. These files are recorded as strict expected failures in the test
harness. If YAMLRocks ever starts accepting one unexpectedly, the suite fails so
the behavior change is visible.

Helm chart templates are excluded for the same reason: files under a chart's
`templates/` directory are Go text/template source and are not YAML documents
until Helm renders them.

## See also

- [Round-trip editing](/guides/round-trip/): the byte-for-byte editing promise.
- [Includes](/guides/includes/): native `!include` resolution and write-back.
- [Performance](/guides/performance/): reproducible benchmark commands.
- [Projects using YAMLRocks](/projects/): actual public adopters.
