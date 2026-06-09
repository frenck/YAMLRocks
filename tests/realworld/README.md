# Real-world configuration tests

This category parses large, public configurations from many ecosystems and
asserts that every YAML file **parses** and **round-trips byte-for-byte** in
`OPT_ROUND_TRIP` mode. It is the strongest real-world signal for the parser: real
configs combine comments, anchors, merge keys, custom tags, multi-document
streams, block scalars, and templating-in-strings in ways a synthetic suite
rarely does.

Two kinds of test run:

- **Per file** (`test_parses_and_round_trips`, every ecosystem): each `*.yaml`/
  `*.yml` file is parsed standalone and must re-emit byte-for-byte. Custom tags
  (`!include`, `!secret`, ...) are preserved, not resolved, so split configs and
  absent secret files do not matter.
- **Include graph** (`test_ha_include_graph_round_trips`, Home Assistant): a
  config is loaded _through_ `configuration.yaml` with `OPT_INCLUDES`, resolving
  the whole `!include` tree, then checked for the two write-back guarantees: the
  root re-emits with its directives restored, and every resolved source file
  re-emits exactly as on disk (unmodified files come straight from their cached
  source, so they are byte-for-byte). A repo whose graph references uncommitted
  files (secrets, generated credentials) is a strict xfail.

## Structure

Configs are pulled in as **git submodules**, kept apart from the test code in the
data tree, organized by ecosystem under `tests/data/realworld/<ecosystem>/<repo>`,
so the corpus grows by adding repos to an ecosystem (or adding a new ecosystem
folder). Submodules reference the upstream repos by pinned commit without copying
third-party files into this repo.

Home Assistant has the deepest coverage (15 configs, since it is the primary
target); every other ecosystem seeds a few repos, and all are easy to grow. In
total the corpus spans **25 ecosystems, 95 repositories, and roughly 22,700 YAML
files**.

The `Files` column is the count of `*.yaml`/`*.yml` files under each ecosystem
(Helm chart templates excluded; see below). `Sources` lists the upstream
`owner/repo` behind each submodule path under `tests/data/realworld/<ecosystem>/`.

| Ecosystem       | Repos | Files | Sources (`owner/repo`)                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| --------------- | ----- | ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Home Assistant  | 15    | 1756  | Bahnburner/Home-Assistant-Config, DubhAd/Home-AssistantConfig, arsaboo/homeassistant-config, bachya/smart-home, basnijholt/home-assistant-config, benct/home-assistant-config, bieniu/home-assistant-config, dshokouhi/Home-AssistantConfig, frenck/home-assistant-config, hmmbob/HomeAssistantConfig, jcallaghan/home-assistant-config, nagyrobi/home-assistant-configuration-examples, renemarc/home-assistant-config, shortbloke/home_assistant_config, thomasloven/hass-config |
| Ansible         | 7     | 674   | dev-sec/ansible-collection-hardening, geerlingguy/ansible-for-devops, geerlingguy/ansible-role-docker, geerlingguy/ansible-role-mysql, geerlingguy/ansible-role-nginx, geerlingguy/mac-dev-playbook, prometheus-community/ansible                                                                                                                                                                                                                                                  |
| ESPHome         | 7     | 3264  | AlexMekkering/esphome-config, athom-tech/esp32-configs, esphome/esphome, esphome/firmware, jesserockz/esphome-configs, landonr/lilygo-tdisplays3-esphome, nrandell/esphome                                                                                                                                                                                                                                                                                                         |
| Kubernetes      | 6     | 1175  | GoogleCloudPlatform/microservices-demo, dockersamples/example-voting-app, kelseyhightower/kubernetes-the-hard-way, kubernetes-sigs/kubespray, kubernetes-sigs/kustomize, kubernetes/examples                                                                                                                                                                                                                                                                                       |
| Docker Compose  | 5     | 493   | Haxxnet/Compose-Examples, compose-spec/compose-spec, docker/awesome-compose, docker/compose, vegasbrianc/prometheus                                                                                                                                                                                                                                                                                                                                                                |
| GitOps          | 5     | 569   | argoproj/argocd-example-apps, fluxcd/flux2, fluxcd/flux2-kustomize-helm-example, rancher/fleet-examples, stefanprodan/podinfo                                                                                                                                                                                                                                                                                                                                                      |
| CircleCI        | 4     | 91    | CircleCI-Public/circleci-demo-go, CircleCI-Public/circleci-demo-javascript-express, CircleCI-Public/circleci-demo-python-django, circleci/circleci-docs                                                                                                                                                                                                                                                                                                                            |
| Helm            | 4     | 2002  | grafana/helm-charts, helm/helm, jenkinsci/helm-charts, prometheus-community/helm-charts                                                                                                                                                                                                                                                                                                                                                                                            |
| OpenAPI         | 4     | 112   | OAI/OpenAPI-Specification, readmeio/oas-examples, stripe/openapi, swagger-api/swagger-petstore                                                                                                                                                                                                                                                                                                                                                                                     |
| Prometheus      | 4     | 713   | prometheus-operator/kube-prometheus, prometheus-operator/prometheus-operator, prometheus/alertmanager, prometheus/prometheus                                                                                                                                                                                                                                                                                                                                                       |
| Argo            | 3     | 3398  | argoproj/argo-cd, argoproj/argo-rollouts, argoproj/argo-workflows                                                                                                                                                                                                                                                                                                                                                                                                                  |
| CloudFormation  | 3     | 397   | aws-cloudformation/aws-cloudformation-templates, awslabs/aws-cloudformation-templates, widdix/aws-cf-templates                                                                                                                                                                                                                                                                                                                                                                     |
| dbt             | 3     | 134   | dbt-labs/dbt-core, dbt-labs/dbt-utils, dbt-labs/jaffle-shop-classic                                                                                                                                                                                                                                                                                                                                                                                                                |
| GitHub Actions  | 3     | 277   | actions/setup-node, actions/starter-workflows, actions/toolkit                                                                                                                                                                                                                                                                                                                                                                                                                     |
| OpenTelemetry   | 3     | 3526  | open-telemetry/opentelemetry-collector, open-telemetry/opentelemetry-collector-contrib, open-telemetry/opentelemetry-demo                                                                                                                                                                                                                                                                                                                                                          |
| Tekton          | 3     | 1787  | tektoncd/catalog, tektoncd/pipeline, tektoncd/triggers                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Azure Pipelines | 2     | 50    | MicrosoftDocs/pipelines-java, microsoft/azure-pipelines-yaml                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Cloud Foundry   | 2     | 354   | cloudfoundry/bosh-deployment, cloudfoundry/cf-deployment                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Concourse       | 2     | 234   | concourse/concourse, concourse/concourse-docker                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Crossplane      | 2     | 371   | crossplane/crossplane, upbound/platform-ref-aws                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| MkDocs          | 2     | 31    | mkdocstrings/mkdocstrings, squidfunk/mkdocs-material                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Serverless      | 2     | 816   | aws-samples/serverless-patterns, serverless/examples                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Woodpecker      | 2     | 130   | woodpecker-ci/plugin-git, woodpecker-ci/woodpecker                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| cloud-init      | 1     | 262   | canonical/cloud-init                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| goss            | 1     | 95    | goss-org/goss                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |

See `.gitmodules` for the exact repo URL behind each path. Each submodule is
pinned to a specific commit (recorded in the parent repo), so the corpus is
reproducible until deliberately bumped.

**Helm chart templates are not collected.** A file under a `templates/`
directory whose chart root (the parent of `templates/`) holds a `Chart.yaml` is
Go text/template (`{{- if ... }}`) that Helm/Argo render _before_ parsing as YAML,
so it is not a standalone YAML document (PyYAML rejects it too).
`_is_chart_template` in `test_realworld.py` keys on that sibling `Chart.yaml`, so
it catches a chart anywhere (e.g. a `helm-chart/` inside a Kubernetes demo) while
leaving genuine YAML in a non-chart `templates/` dir, such as CloudFormation
templates, which _are_ valid YAML.

## Running

```sh
# One-time: fetch the configs.
git submodule update --init

# Run the whole category (each file is its own test, so failures name the file).
pytest tests/realworld -m realworld

# Just one ecosystem (matches the test id ecosystem/repo/path).
pytest tests/realworld -k ansible
```

Without the submodules checked out the category **auto-skips**, so a plain
`pytest` stays green for contributors who did not fetch them.

## Adding repos and ecosystems

1. Add a submodule under the right ecosystem (create the folder for a new one):
   `git submodule add --depth 1 https://github.com/<owner>/<repo>.git tests/data/realworld/<ecosystem>/<name>`
2. Prefer license-clear, moderately sized repos; submodules are cloned in CI, so
   very large histories slow the run.
3. Run the suite. Any file that is **not valid standalone YAML** (e.g. a Jinja or
   Helm template, or a config with a spec-invalid construct) goes in
   `KNOWN_INVALID` in `test_realworld.py` with a one-line reason. These are
   _strict_ xfails: if the parser ever starts accepting one, the suite fails so
   the change is noticed.
4. A genuine parse or round-trip failure on valid YAML is a **parser bug**: fix
   the parser, never add the file to `KNOWN_INVALID` to silence it.

To refresh to the latest upstream commits:
`git submodule update --remote` then commit the new pointers.
