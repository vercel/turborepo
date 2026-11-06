from hatchling.metadata.plugin.interface import MetadataHookInterface


class CustomMetadataHook(MetadataHookInterface):
    def update(self, metadata):
        metadata["dependencies"] = ["py-lib[feature]"]
        metadata["optional-dependencies"] = {"feature": ["feature-lib"]}
