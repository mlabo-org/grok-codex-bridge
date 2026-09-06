require 'minitest/autorun'
require 'tmpdir'
require_relative '../scripts/uninstall-native'

class NativeUninstallTest < Minitest::Test
  def setup
    @root = '/Users/example/Library/Application Support/grok-codex-bridge'
    @native = { 'model' => 'gpt-6-astra', 'model_provider' => 'openai',
      'model_providers' => NativeUninstall::LEGACY.to_h { |id| [id, NativeUninstall::DIRECT_OPENAI.dup] },
      'features' => { 'context_management' => { 'experimental_mode' => true } },
      'mcp_servers' => { 'retained' => { 'command' => '/retained/tool' } } }
  end

  def test_standard_defaults_and_unrelated_settings_are_accepted_without_mutation
    original = Marshal.load(Marshal.dump(@native))
    assert NativeUninstall.verify_config(@native, @root)
    assert_equal original, @native
    assert NativeUninstall.verify_config({}, @root, require_aliases: false)
    assert_raises(RuntimeError) { NativeUninstall.verify_config({}, @root) }
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

  def test_saved_provider_must_not_retain_bridge_transport_or_credentials
    [
      { 'base_url' => 'http://127.0.0.1:8746/v1' },
      { 'requires_openai_auth' => false },
      { 'http_headers' => { 'X-Grok-Capability' => 'test-only' } }
    ].each do |override|
      config = Marshal.load(Marshal.dump(@native))
      config['model_providers']['grok_codex_picker'].merge!(override)
      assert_raises(RuntimeError) { NativeUninstall.verify_config(config, @root) }
    end
  end

  def test_official_config_write_repairs_missing_provider_without_changing_defaults_or_other_settings
    skip 'Requires the app-bundled Codex' unless File.executable?(NativeUninstall::APP_CODEX)
    Dir.mktmpdir('grok-uninstall-history-') do |home|
      path = File.join(home, 'config.toml')
      File.write(path, <<~TOML)
        # Preserve the user's settings.
        model = "gpt-6-astra"
        model_reasoning_effort = "low"
        [features]
        multi_agent = false
        [model_providers.unrelated]
        name = "Retained"
        base_url = "http://127.0.0.1:12345/v1"
      TOML
      env = { 'CODEX_HOME' => home }
      args = [NativeUninstall::APP_CODEX, '-c', 'model_provider="grok_codex_picker"', 'features', 'list']
      output, status = Open3.capture2e(env, *args)
      refute status.success?
      assert_includes output, 'Model provider `grok_codex_picker` not found'
      connection = NativeUninstall::AppServer.new(home)
      begin
        before = connection.call('config/read', includeLayers: false).fetch('config')
        assert NativeUninstall.restore_saved_providers(connection, home, @root)
        after = connection.call('config/read', includeLayers: false).fetch('config')
        NativeUninstall::LEGACY.each { |id| after.fetch('model_providers').delete(id) }
        assert_equal before, after
        assert_includes File.read(path), '# Preserve the user\'s settings.'
        output, status = Open3.capture2e(env, *args)
        assert status.success?, output
      ensure
        connection.close
      end
    end
  end
end
