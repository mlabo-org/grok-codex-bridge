require 'minitest/autorun'
require_relative '../scripts/uninstall-native'

class NativeUninstallTest < Minitest::Test
  def setup
    @root = '/Users/example/Library/Application Support/grok-codex-bridge'
    @native = { 'model' => 'gpt-6-astra', 'model_provider' => 'openai',
      'features' => { 'context_management' => { 'experimental_mode' => true } },
      'mcp_servers' => { 'retained' => { 'command' => '/retained/tool' } } }
  end

  def test_standard_defaults_and_unrelated_settings_are_accepted_without_mutation
    original = Marshal.load(Marshal.dump(@native))
    assert NativeUninstall.verify_config(@native, @root)
    assert_equal original, @native
    assert NativeUninstall.verify_config({}, @root)
  end

  def test_native_compatibility_mode_is_not_accepted_as_removal
    assert_raises(RuntimeError) do
      NativeUninstall.verify_config(@native.merge('model_provider' => 'grok_codex_picker'), @root)
    end
    NativeUninstall::LEGACY.each do |id|
      assert_raises(RuntimeError) do
        NativeUninstall.verify_config(@native.merge('model_providers' => { id => { 'name' => 'OpenAI' } }), @root)
      end
    end
    assert_raises(RuntimeError) do
      NativeUninstall.verify_config(@native.merge('model_catalog_json' => @root + '/state/models.json'), @root)
    end
  end

  def test_proxy_and_grok_defaults_cannot_pass_as_native
    [
      { 'model' => 'grok-4.6' },
      { 'openai_base_url' => 'http://127.0.0.1:8746/v1' },
      { 'model_providers' => { 'openai' => { 'base_url' => 'http://127.0.0.1:8746/v1' } } }
    ].each do |override|
      assert_raises(RuntimeError) { NativeUninstall.verify_config(@native.merge(override), @root) }
    end
  end
end
