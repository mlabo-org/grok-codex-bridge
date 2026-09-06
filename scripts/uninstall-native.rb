#!/usr/bin/ruby
# Remove the bridge while resolving saved provider IDs directly through OpenAI.
require 'json'
require 'open3'
require 'timeout'
require 'shellwords'
require 'socket'

module NativeUninstall
  APP_CODEX = '/Applications/ChatGPT.app/Contents/Resources/codex'.freeze
  LEGACY = %w[grok_codex_picker grok_bridge].freeze
  DIRECT_OPENAI = {
    'name' => 'OpenAI',
    'base_url' => 'https://chatgpt.com/backend-api/codex',
    'wire_api' => 'responses',
    'requires_openai_auth' => true,
    'supports_websockets' => true
  }.freeze

  def self.verify_config(config, root, require_aliases: true)
    raise 'Default provider is not the built-in OpenAI provider' unless (config['model_provider'] || 'openai') == 'openai'
    raise 'Default model is still Grok' if config['model'].to_s.start_with?('grok-')
    providers = config['model_providers'] || {}
    LEGACY.each do |id|
      provider = providers[id]
      next if !require_aliases && !provider
      # config/read includes unset fields as null and the disabled search default.
      explicit = provider&.reject { |key, value| value.nil? || (key == 'supports_standalone_web_search' && value == false) }
      raise "Saved provider #{id} must connect directly to OpenAI" unless explicit == DIRECT_OPENAI
    end
    raise 'The built-in OpenAI provider is overridden' if providers.key?('openai')
    raise 'The OpenAI base URL is overridden' if config['openai_base_url']
    catalog = config['model_catalog_json']
    raise 'The bridge model catalog remains configured' if catalog && File.expand_path(catalog).start_with?(root + '/')
    true
  end

  def self.restore_saved_providers(connection, codex_home, root)
    snapshot = connection.call('config/read', includeLayers: true)
    verify_config(snapshot.fetch('config'), root, require_aliases: false)
    config_path = File.realpath(File.join(codex_home, 'config.toml'))
    layer = snapshot.fetch('layers').find do |entry|
      entry.dig('name', 'type') == 'user' && entry.dig('name', 'file') == config_path
    end
    raise 'Cannot identify the user config version' unless layer
    result = connection.call('config/batchWrite', filePath: config_path,
      expectedVersion: layer.fetch('version'), reloadUserConfig: false,
      edits: LEGACY.map do |id|
        { keyPath: "model_providers.#{id}", value: DIRECT_OPENAI, mergeStrategy: 'replace' }
      end)
    raise 'Direct OpenAI provider settings were overridden' unless result['status'] == 'ok'
    config = connection.call('config/read', includeLayers: false).fetch('config')
    verify_config(config, root)
  end

  class AppServer
    def initialize(codex_home)
      # No provider/model override: verify the actual restored user defaults.
      @input, @output, @errors, @wait = Open3.popen3(
        { 'CODEX_HOME' => codex_home }, APP_CODEX, 'app-server')
      @drain = Thread.new { @errors.each_line { |_| } }
      @sequence = 0
      call('initialize', clientInfo: { name: 'grok_bridge_native_uninstall', version: '1.0' },
                         capabilities: { experimentalApi: true })
      @input.puts(JSON.generate(method: 'initialized'))
      @input.flush
    end

    def call(method, params)
      @sequence += 1
      @input.puts(JSON.generate(id: @sequence, method: method, params: params))
      @input.flush
      Timeout.timeout(30) do
        loop do
          item = JSON.parse(@output.gets || raise('Codex App Server closed'))
          next unless item['id'] == @sequence
          # Backend error text may include private config or prompt values.
          raise "Codex rejected #{method} (#{item.dig('error', 'code')})" if item['error']
          return item.fetch('result')
        end
      end
    end

    def smoke(cwd)
      # Ephemeral official thread: no existing thread or database is rewritten.
      started = call('thread/start', cwd: cwd, ephemeral: true,
        approvalPolicy: 'never', sandbox: 'read-only')
      raise 'New thread did not use OpenAI' unless started['modelProvider'] == 'openai'
      id = started.fetch('thread').fetch('id')
      call('turn/start', threadId: id, effort: 'low', input: [
        { type: 'text', text: 'Reply with NATIVE_RESTORE_OK only. Do not call any tools.' }
      ])
      Timeout.timeout(120) do
        loop do
          item = JSON.parse(@output.gets || raise('Codex App Server closed during native proof'))
          next unless item['method'] == 'turn/completed' && item.dig('params', 'threadId') == id
          raise 'Direct ChatGPT OAuth inference did not complete' unless item.dig('params', 'turn', 'status') == 'completed'
          return started.fetch('model')
        end
      end
    end

    def close
      @input.close unless @input.closed?
      Timeout.timeout(8) { @wait.value }
    rescue Timeout::Error
      Process.kill('TERM', @wait.pid) if @wait.alive?
    ensure
      @drain.join(1) if @drain
    end
  end

  class Operation
    def initialize
      @home = Dir.home
      @codex_home = File.expand_path(ENV.fetch('CODEX_HOME', File.join(@home, '.codex')))
      @root = File.join(@home, 'Library/Application Support/grok-codex-bridge')
      @bridge = File.join(@root, 'bin/grok-codex-bridge')
      @profile = File.join(@codex_home, 'grok-bridge.config.toml')
      @agent = File.join(@home, 'Library/LaunchAgents/com.local.grok-codex-bridge.plist')
      @restart = File.join(@home, '.local/bin/codex-remote-restart')
      @log = File.join(@home, 'Library/Logs/grok-codex-uninstall.log')
    end

    def command(*args)
      output, status = Open3.capture2e(*args)
      raise "Command failed: #{File.basename(args.first)} #{args[1]} (exit #{status.exitstatus})" unless status.success?
      output
    end

    def regular(path)
      raise "Missing or unsafe file: #{path}" unless File.file?(path) && !File.symlink?(path)
    end

    def preflight(restart = false)
      regular(APP_CODEX); regular(@bridge); regular(File.join(@codex_home, 'config.toml'))
      regular(File.join(@root, 'install-manifest.json'))
      regular(@restart) if restart
      raise 'Unsafe installation root' if File.symlink?(@root) || File.symlink?(@codex_home)
      manifest = JSON.parse(File.read(File.join(@root, 'install-manifest.json')))
      raise 'Installation manifest points elsewhere' unless manifest['install_root'] == @root &&
        manifest['profile_path'] == @profile && manifest['launch_agent_path'] == @agent
      # The existing native uninstaller also validates exact picker rollback
      # ownership before stopping launchd. No ad-hoc launchctl or deletion here.
      command(@bridge, 'doctor', '--native-compatibility', '--codex-home', @codex_home)
      raise 'ChatGPT OAuth login is required' unless command(APP_CODEX, 'login', 'status').strip == 'Logged in using ChatGPT'
      @bind = manifest.fetch('bind')
      @profile_created = manifest.fetch('profile_created')
      @agent_created = manifest.fetch('launch_agent_created')
      puts 'Preflight passed: installed bridge ownership and ChatGPT OAuth login.'
      puts 'Removal includes bridge runtime, managed picker settings and service; source and saved conversations are retained.'
      puts 'Saved provider IDs will connect directly to OpenAI. History is not migrated; saved Grok models require selecting a GPT model.'
    end

    def execute(restart)
      preflight(restart)
      removed = false
      begin
        puts command(@bridge, 'uninstall', '--codex-home', @codex_home)
        removed = true
        raise 'Bridge runtime still exists' if File.exist?(@root)
        raise 'Bridge profile still exists' if @profile_created && File.exist?(@profile)
        raise 'Bridge LaunchAgent still exists' if @agent_created && File.exist?(@agent)
        host, port = @bind.match(/\A(127\.0\.0\.1|\[::1\]):(\d+)\z/)&.captures
        raise 'Cannot validate the removed loopback listener' unless host
        begin
          socket = Socket.tcp(host.delete('[]'), Integer(port), connect_timeout: 2)
        rescue Errno::ECONNREFUSED
          socket = nil
        end
        if socket
          socket.close
          raise 'A process is still listening on the bridge port'
        end
        connection = AppServer.new(@codex_home)
        NativeUninstall.restore_saved_providers(connection, @codex_home, @root)
        puts 'Verified: built-in OpenAI defaults, saved provider IDs connect directly to OpenAI, no bridge catalog/URL/listener.'
        puts "Verified: direct ChatGPT OAuth inference completed with #{connection.smoke(@home)}."
        puts 'Native restoration: complete. Existing in-memory sessions still require an app restart.'
      ensure
        connection.close if connection
        # Even a failed network smoke must not strand the desktop on its old,
        # already removed bridge connection. Restart status is reported separately.
        if restart && removed
          puts command(@restart, '--execute')
          puts 'Desktop restart: handed off; reopened UI is verified after restart.'
        end
      end
    end

    def handoff
      preflight(true)
      regular(File.expand_path(__FILE__))
      raise 'Unsafe log target' if File.symlink?(@log)
      File.open(@log, File::WRONLY | File::CREAT | File::APPEND, 0600) { |f| f.puts("#{Time.now.utc} native uninstall requested") }
      # Terminal owns the detached process so removal can finish after the
      # current Codex turn loses its old provider connection.
      # Let the requesting Codex turn finish its handoff message before its
      # provider disappears. This is one bounded delay, not a background loop.
      argv = ['/bin/sh', '-c', 'sleep 15; exec "$@"', 'native-uninstall',
              '/usr/bin/ruby', File.expand_path(__FILE__), '--execute', '--restart']
      launch = "CODEX_HOME=#{Shellwords.escape(@codex_home)} /usr/bin/nohup #{Shellwords.join(argv)} >>#{Shellwords.escape(@log)} 2>&1 </dev/null & exit"
      applescript = 'tell application "Terminal" to do script ' + JSON.generate(launch)
      command('/usr/bin/osascript', '-e', applescript)
      puts "Uninstall and restart handed off to Terminal. Result log: #{@log}"
    end
  end
end

if $PROGRAM_NAME == __FILE__
  $stdout.sync = true
  begin
    op = NativeUninstall::Operation.new
    mode = ARGV.shift || '--check'
    restart = ARGV.delete('--restart')
    raise 'Unexpected arguments' unless ARGV.empty?
    case mode
    when '--check'
      op.preflight(!!restart)
    when '--execute'
      op.execute(!!restart)
    when '--handoff'
      op.handoff
    else
      raise 'Usage: uninstall-native.rb --check | --execute [--restart] | --handoff'
    end
  rescue => error
    warn "Native uninstall stopped: #{error.message}"
    exit 1
  end
end
